//! Strict JSONL trace. One JSON object per line, `kind` discriminates:
//!
//! | kind | content |
//! |---|---|
//! | `session_start` | policy, sizing, buckets, buy-in, seed, crate config |
//! | `raw` | every websocket message in (`dir:"in"`) and out (`dir:"out"`) |
//! | `decision` | state summary, server legal set, engine/server consistency issues, our full distribution, sampled `u`, chosen nlhe action, mapped Open Poker action + fidelity, latency |
//! | `ack` / `reject` | server's verdict on an action |
//! | `anomaly` | desync, unparsable message, reconnect, rebuy, anything surprising |
//! | `hand` | per-hand result (see [`crate::analyze::HandRecord`]), the analyzer's input |
//! | `session_end` | counters |
//!
//! Every line carries `t` (unix ms). Lines are flushed immediately so a crash loses nothing.

use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub trait TraceSink: Send {
    fn emit(&mut self, rec: Value);
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub struct FileSink {
    w: BufWriter<File>,
}

impl FileSink {
    pub fn create(path: &Path) -> std::io::Result<FileSink> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        let f = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(FileSink { w: BufWriter::new(f) })
    }
}

impl TraceSink for FileSink {
    fn emit(&mut self, mut rec: Value) {
        if let Value::Object(m) = &mut rec {
            m.entry("t").or_insert(Value::from(now_ms()));
        }
        let _ = writeln!(self.w, "{rec}");
        let _ = self.w.flush();
    }
}

/// In-memory sink for tests (clone the handle to read it back).
#[derive(Clone, Default)]
pub struct MemSink(pub Arc<Mutex<Vec<Value>>>);

impl TraceSink for MemSink {
    fn emit(&mut self, rec: Value) {
        self.0.lock().unwrap().push(rec);
    }
}

impl MemSink {
    pub fn records(&self) -> Vec<Value> {
        self.0.lock().unwrap().clone()
    }
    pub fn of_kind(&self, kind: &str) -> Vec<Value> {
        self.records()
            .into_iter()
            .filter(|r| r.get("kind").and_then(Value::as_str) == Some(kind))
            .collect()
    }
}

pub struct NullSink;
impl TraceSink for NullSink {
    fn emit(&mut self, _rec: Value) {}
}
