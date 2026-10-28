//! WebSocket `/v1/play` (TABLE_API.md section 4).

use crate::error::ApiError;
use crate::state::{bearer, AppState, Principal};
use crate::table::TableCmd;
use crate::util::now_ms;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

pub async fn play(
    State(app): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> Response {
    let token = q.get("token").cloned().or_else(|| bearer(&headers));
    let who = match token {
        Some(t) => app.authenticate_token(&t),
        None => Err(ApiError::unauthorized()),
    };
    match who {
        Ok(Principal::Player(p)) => ws.on_upgrade(move |s| conn(app, p.player_id, p.name, s)),
        Ok(Principal::Admin) => ApiError::forbidden("the admin token cannot play; use a bot token").into_response(),
        Err(e) => e.into_response(),
    }
}

fn err_msg(code: &str, message: &str, details: Value) -> String {
    json!({"v": 1, "type": "error", "t_ms": now_ms(), "data": {"code": code, "message": message, "details": details}}).to_string()
}

async fn conn(app: Arc<AppState>, pid: String, name: String, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = unbounded_channel::<String>();
    let conn_id = app.hub.register(&pid, tx.clone());
    let writer = tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            if sink.send(Message::Text(m.into())).await.is_err() {
                break;
            }
        }
    });
    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            Message::Text(t) => handle(&app, &pid, &name, &tx, t.as_str()),
            Message::Close(_) => break,
            _ => {}
        }
    }
    app.hub.unregister(&pid, conn_id);
    writer.abort();
}

fn handle(app: &Arc<AppState>, pid: &str, name: &str, tx: &UnboundedSender<String>, text: &str) {
    let Ok(m) = serde_json::from_str::<Value>(text) else {
        let _ = tx.send(err_msg("bad_request", "message is not valid JSON", json!({})));
        return;
    };
    let data = m.get("data").cloned().unwrap_or(json!({}));
    match m["type"].as_str().unwrap_or("") {
        "hello" => {
            let tables = app.tables_of(pid);
            let ids: Vec<&str> = tables.iter().map(|t| t.id.as_str()).collect();
            let _ = tx.send(
                json!({"v": 1, "type": "welcome", "t_ms": now_ms(),
                       "data": {"player_id": pid, "name": name, "server_time": now_ms(), "tables": ids, "protocol": 1}})
                .to_string(),
            );
            for t in &tables {
                // Authoritative snapshot (no event replay), then the pending your_turn if any.
                let _ = tx.send(t.snapshot_msg(&app.hub, pid));
                let _ = t.tx.send(TableCmd::Resume { player_id: pid.to_string(), conn: tx.clone() });
            }
        }
        "ping" => {
            let _ = tx.send(json!({"v": 1, "type": "pong", "t_ms": now_ms(), "data": {}}).to_string());
        }
        "act" => {
            let table = match m["table_id"].as_str() {
                Some(id) => app.table(id).ok(),
                None => {
                    let hid = data["hand_id"].as_str().unwrap_or("");
                    let ts = app.tables_of(pid);
                    ts.iter()
                        .find(|t| t.inner.lock().unwrap().hand_id.as_deref() == Some(hid))
                        .cloned()
                        .or_else(|| if ts.len() == 1 { ts.first().cloned() } else { None })
                }
            };
            match table {
                Some(t) => {
                    let _ = t.tx.send(TableCmd::Act { player_id: pid.to_string(), data, conn: tx.clone() });
                }
                None => {
                    let _ = tx.send(err_msg("not_seated", "you are not seated at that table", json!({})));
                }
            }
        }
        "leave" => {
            if let Some(t) = data["table_id"].as_str().or(m["table_id"].as_str()).and_then(|id| app.table(id).ok()) {
                let _ = t.tx.send(TableCmd::Leave { player_id: pid.to_string() });
            } else {
                let _ = tx.send(err_msg("not_seated", "unknown table", json!({})));
            }
        }
        other => {
            let _ = tx.send(err_msg("bad_request", &format!("unknown message type {other:?}"), json!({})));
        }
    }
}
