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
    if let Err(e) = app.ensure_accepting() {
        return e.into_response();
    }
    let who = match token {
        Some(t) => app.authenticate_token(&t),
        None => Err(ApiError::unauthorized()),
    };
    match who {
        Ok(Principal::Player(p)) => {
            let max = app.cfg.max_ws_per_player;
            if max > 0 && app.hub.count(&p.player_id) >= max {
                return ws_limit(max).into_response();
            }
            let mx = app.cfg.max_ws_message_bytes;
            ws.max_message_size(mx).max_frame_size(mx).on_upgrade(move |s| conn(app, p.player_id, p.name, s))
        }
        Ok(Principal::Admin | Principal::Runner) => ApiError::forbidden("only a bot token can play; use a bot token").into_response(),
        Err(e) => e.into_response(),
    }
}

async fn wait_sd(rx: &mut tokio::sync::watch::Receiver<bool>) {
    let _ = rx.wait_for(|v| *v).await.map(|_| ());
}

fn ws_limit(max: usize) -> ApiError {
    ApiError::rate_limited(
        format!("this player already has {max} open sockets (limit)"),
        json!({"limit": "ARENA_MAX_WS_PER_PLAYER", "max": max}),
    )
}

fn err_msg(code: &str, message: &str, details: Value) -> String {
    json!({"v": 1, "type": "error", "t_ms": now_ms(), "data": {"code": code, "message": message, "details": details}}).to_string()
}

async fn conn(app: Arc<AppState>, pid: String, name: String, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = unbounded_channel::<String>();
    let Some(conn_id) = app.hub.try_register(&pid, tx.clone(), app.cfg.max_ws_per_player) else {
        // Lost a race with another connect: same 429 contract, over the socket.
        let e = ws_limit(app.cfg.max_ws_per_player);
        let _ = sink.send(Message::Text(err_msg(e.code, &e.message, e.details).into())).await;
        let _ = sink.send(Message::Close(None)).await;
        return;
    };
    let mut sd_w = app.shutdown.subscribe();
    let writer = tokio::spawn(async move {
        loop {
            tokio::select! {
                m = rx.recv() => match m {
                    Some(m) => {
                        if sink.send(Message::Text(m.into())).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                },
                _ = wait_sd(&mut sd_w) => {
                    let _ = sink.send(Message::Close(None)).await;
                    break;
                }
            }
        }
    });
    let mut sd_r = app.shutdown.subscribe();
    loop {
        let msg = tokio::select! {
            m = stream.next() => match m { Some(Ok(m)) => m, _ => break },
            _ = wait_sd(&mut sd_r) => {
                // let the writer deliver the close frame
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                break;
            }
        };
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
