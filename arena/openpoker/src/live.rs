//! Network IO: websocket loop with reconnect, REST helpers, API-key loading.
//!
//! Nothing here is reachable from tests; all logic worth testing is in [`crate::session`].
//! The API key is read from `OPENPOKER_API_KEY` or a `KEY=VALUE` file and is never printed or
//! traced (it only appears in the `Authorization` header).

use crate::session::Session;
use crate::trace::now_ms;
use serde_json::{json, Value};
use std::net::TcpStream;
use std::path::Path;
use std::time::{Duration, Instant};
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};

pub const WS_URL: &str = "wss://openpoker.ai/ws";
pub const API_URL: &str = "https://api.openpoker.ai";

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

pub struct LiveOpts {
    pub ws_url: String,
    pub api_url: String,
    pub api_key: String,
    /// Wall-clock cap for the whole session.
    pub timeout: Duration,
    pub max_reconnects: u32,
}

/// Look for the key in the environment, then in the given file, then in the usual places.
pub fn load_api_key(file: Option<&Path>) -> Result<String, String> {
    if let Ok(v) = std::env::var("OPENPOKER_API_KEY") {
        if !v.trim().is_empty() {
            return Ok(v.trim().to_string());
        }
    }
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Some(f) = file {
        candidates.push(f.to_path_buf());
    }
    candidates.push(".env.openpoker".into());
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(Path::new(&home).join("pluribus/.env.openpoker"));
    }
    candidates.push(".env".into());
    for c in &candidates {
        if let Ok(text) = std::fs::read_to_string(c) {
            if let Some(k) = parse_env_key(&text) {
                return Ok(k);
            }
        }
    }
    Err("OPENPOKER_API_KEY not found (env var, --env FILE, ./.env.openpoker, ~/pluribus/.env.openpoker)".into())
}

pub fn parse_env_key(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim().trim_start_matches("export ");
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == "OPENPOKER_API_KEY" {
                let v = v.trim().trim_matches('"').trim_matches('\'').trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

fn connect_ws(opts: &LiveOpts) -> Result<Socket, String> {
    let mut req = opts
        .ws_url
        .as_str()
        .into_client_request()
        .map_err(|e| format!("bad websocket url: {e}"))?;
    req.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", opts.api_key).parse().map_err(|e| format!("auth header: {e}"))?,
    );
    let (sock, _) = connect(req).map_err(|e| format!("websocket connect failed: {e}"))?;
    set_read_timeout(&sock, Duration::from_secs(5));
    Ok(sock)
}

fn set_read_timeout(sock: &Socket, d: Duration) {
    match sock.get_ref() {
        MaybeTlsStream::Plain(s) => {
            let _ = s.set_read_timeout(Some(d));
        }
        MaybeTlsStream::Rustls(s) => {
            let _ = s.get_ref().set_read_timeout(Some(d));
        }
        _ => {}
    }
}

fn send(sock: &mut Socket, v: &Value) -> Result<(), String> {
    sock.send(Message::Text(v.to_string())).map_err(|e| format!("send failed: {e}"))
}

fn get_json(opts: &LiveOpts, path: &str) -> Result<Value, String> {
    get_json_url(&opts.api_key, &format!("{}{}", opts.api_url.trim_end_matches('/'), path))
}

fn get_json_url(key: &str, url: &str) -> Result<Value, String> {
    ureq::get(url)
        .set("Authorization", &format!("Bearer {key}"))
        .timeout(Duration::from_secs(20))
        .call()
        .map_err(|e| format!("GET {url}: {e}"))?
        .into_json::<Value>()
        .map_err(|e| format!("GET {url}: not JSON: {e}"))
}

pub fn fetch_history(api_key: &str, api_url: &str, limit: usize, offset: usize) -> Result<Value, String> {
    get_json_url(
        api_key,
        &format!("{}/api/me/hand-history/export?format=json&limit={limit}&offset={offset}", api_url.trim_end_matches('/')),
    )
}

pub fn fetch_season_me(api_key: &str, api_url: &str) -> Result<Value, String> {
    get_json_url(api_key, &format!("{}/api/season/me", api_url.trim_end_matches('/')))
}

/// Run until `session.done`, the wall-clock timeout, a fatal error, or reconnects run out.
/// Returns the reason the loop ended.
pub fn run(opts: &LiveOpts, session: &mut Session) -> Result<String, String> {
    let deadline = Instant::now() + opts.timeout;
    let mut reconnects = 0u32;
    let mut first = true;
    let reason: String = 'outer: loop {
        if let Some(f) = &session.fatal {
            break f.clone();
        }
        if Instant::now() >= deadline {
            break "timeout".into();
        }
        let mut sock = match connect_ws(opts) {
            Ok(s) => s,
            Err(e) => {
                reconnects += 1;
                session_anomaly(session, "connect_failed", &e);
                if reconnects > opts.max_reconnects {
                    return Err(format!("giving up after {reconnects} failed connects: {e}"));
                }
                std::thread::sleep(Duration::from_secs((2u64 << reconnects.min(5)).min(30)));
                continue;
            }
        };
        // Initial join, or reconcile with the server after a reconnect.
        let init: Vec<Value> = if first {
            session.join_messages()
        } else {
            match get_json(opts, "/api/me/active-game") {
                Ok(v) if v.get("playing").and_then(Value::as_bool) == Some(true) => {
                    let tid = v
                        .get("table_id")
                        .and_then(Value::as_str)
                        .or_else(|| v.pointer("/table/table_id").and_then(Value::as_str))
                        .map(str::to_string);
                    session_anomaly(session, "reconnect_resync", &format!("table {tid:?}"));
                    session.resync_message(tid.as_deref()).into_iter().collect()
                }
                Ok(_) => session.join_messages(),
                Err(e) => {
                    session_anomaly(session, "active_game_failed", &e);
                    session.join_messages()
                }
            }
        };
        first = false;
        for m in &init {
            if let Err(e) = send(&mut sock, m) {
                session_anomaly(session, "send_failed", &e);
            }
        }
        let mut left_at: Option<Instant> = None;
        loop {
            if session.done && left_at.is_none() {
                left_at = Some(Instant::now());
            }
            if let Some(t) = left_at {
                // Give the server a moment to confirm leave_table, then close.
                if t.elapsed() > Duration::from_secs(3) {
                    let _ = sock.close(None);
                    break 'outer if let Some(f) = &session.fatal { f.clone() } else { "done".to_string() };
                }
            } else if Instant::now() >= deadline {
                for m in session.leave_messages() {
                    let _ = send(&mut sock, &m);
                }
                std::thread::sleep(Duration::from_millis(500));
                let _ = sock.close(None);
                break 'outer "timeout".into();
            }
            match sock.read() {
                Ok(Message::Text(t)) => {
                    let Ok(v) = serde_json::from_str::<Value>(&t) else {
                        session_anomaly(session, "invalid_json", &t.chars().take(200).collect::<String>());
                        continue;
                    };
                    let out = session.on_message(&v);
                    let mut failed = false;
                    for m in out {
                        if let Err(e) = send(&mut sock, &m) {
                            session_anomaly(session, "send_failed", &e);
                            failed = true;
                            break;
                        }
                    }
                    if failed {
                        break;
                    }
                    if session.fatal.is_some() {
                        let _ = sock.close(None);
                        break 'outer session.fatal.clone().unwrap();
                    }
                }
                Ok(Message::Close(c)) => {
                    session_anomaly(session, "closed_by_server", &format!("{c:?}"));
                    if session.done {
                        break 'outer "done".into();
                    }
                    break;
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) =>
                {
                    let _ = sock.flush();
                    for m in session.on_idle() {
                        if send(&mut sock, &m).is_err() {
                            break;
                        }
                    }
                }
                Err(e) => {
                    session_anomaly(session, "read_failed", &e.to_string());
                    if session.done {
                        break 'outer "done".into();
                    }
                    break;
                }
            }
        }
        // Dropped out of the read loop: reconnect.
        reconnects += 1;
        if reconnects > opts.max_reconnects {
            return Err(format!("giving up after {reconnects} reconnects"));
        }
        std::thread::sleep(Duration::from_secs((1u64 << reconnects.min(4)).min(15)));
    };
    session.finish(&reason);
    Ok(reason)
}

fn session_anomaly(session: &mut Session, what: &str, details: &str) {
    session.note(json!({"kind": "anomaly", "what": what, "details": details, "t": now_ms()}));
}
