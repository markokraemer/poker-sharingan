//! JSON-lines logging to stderr. One object per line: `{"ts_ms","level","event",...fields}`.

use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::{SystemTime, UNIX_EPOCH};

static ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Relaxed);
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn emit(level: &str, event: &str, fields: Value) {
    if !ENABLED.load(Relaxed) {
        return;
    }
    let mut o = json!({"ts_ms": now_ms(), "level": level, "event": event});
    if let (Some(m), Value::Object(f)) = (o.as_object_mut(), fields) {
        for (k, v) in f {
            m.insert(k, v);
        }
    }
    eprintln!("{o}");
}

pub fn info(event: &str, fields: Value) {
    emit("info", event, fields)
}
pub fn warn(event: &str, fields: Value) {
    emit("warn", event, fields)
}
pub fn error(event: &str, fields: Value) {
    emit("error", event, fields)
}
