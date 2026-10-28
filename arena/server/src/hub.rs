//! Registry of live WebSocket connections, by player.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tokio::sync::mpsc::UnboundedSender;

pub type ConnTx = UnboundedSender<String>;

#[derive(Default)]
pub struct Hub {
    conns: Mutex<HashMap<String, Vec<(u64, ConnTx)>>>,
    next: AtomicU64,
}

impl Hub {
    pub fn register(&self, player_id: &str, tx: ConnTx) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.conns.lock().unwrap().entry(player_id.to_string()).or_default().push((id, tx));
        id
    }
    /// Register unless the player already has `max` live connections (`max == 0`: unlimited).
    pub fn try_register(&self, player_id: &str, tx: ConnTx, max: usize) -> Option<u64> {
        let mut g = self.conns.lock().unwrap();
        let v = g.entry(player_id.to_string()).or_default();
        v.retain(|(_, t)| !t.is_closed());
        if max > 0 && v.len() >= max {
            if v.is_empty() {
                g.remove(player_id);
            }
            return None;
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        v.push((id, tx));
        Some(id)
    }
    /// Live connections of one player.
    pub fn count(&self, player_id: &str) -> usize {
        self.conns.lock().unwrap().get(player_id).map(|v| v.iter().filter(|(_, t)| !t.is_closed()).count()).unwrap_or(0)
    }
    /// Live connections of everybody.
    pub fn total(&self) -> usize {
        self.conns.lock().unwrap().values().map(|v| v.iter().filter(|(_, t)| !t.is_closed()).count()).sum()
    }
    pub fn unregister(&self, player_id: &str, conn_id: u64) {
        let mut g = self.conns.lock().unwrap();
        if let Some(v) = g.get_mut(player_id) {
            v.retain(|(i, _)| *i != conn_id);
            if v.is_empty() {
                g.remove(player_id);
            }
        }
    }
    pub fn connected(&self, player_id: &str) -> bool {
        self.conns.lock().unwrap().get(player_id).map(|v| v.iter().any(|(_, t)| !t.is_closed())).unwrap_or(false)
    }
    /// Send a serialized envelope to every live connection of the player.
    pub fn send(&self, player_id: &str, msg: &str) {
        if let Some(v) = self.conns.lock().unwrap().get(player_id) {
            for (_, tx) in v {
                let _ = tx.send(msg.to_string());
            }
        }
    }
}
