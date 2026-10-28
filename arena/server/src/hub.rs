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
