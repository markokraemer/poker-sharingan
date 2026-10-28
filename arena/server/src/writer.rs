//! Background batch writer for hand histories: the hand loop never waits on SQLite.
//! `flush()` is a barrier for readers (REST hand reads, AIVAT at match end).

use crate::model::HandHistory;
use crate::store::Store;
use serde_json::Value;
use std::sync::mpsc::{channel, Sender, TryRecvError};
use std::sync::Arc;

enum Msg {
    Hand(Box<(HandHistory, Vec<Value>)>),
    Flush(tokio::sync::oneshot::Sender<()>),
}

pub struct HandWriter {
    tx: Sender<Msg>,
}

impl HandWriter {
    pub fn spawn(store: Arc<dyn Store>) -> Self {
        let (tx, rx) = channel::<Msg>();
        std::thread::Builder::new()
            .name("hand-writer".into())
            .spawn(move || {
                let mut batch: Vec<(HandHistory, Vec<Value>)> = Vec::new();
                let mut flushes: Vec<tokio::sync::oneshot::Sender<()>> = Vec::new();
                loop {
                    let first = match rx.recv() {
                        Ok(m) => m,
                        Err(_) => break,
                    };
                    let mut next = Some(first);
                    while let Some(m) = next.take() {
                        match m {
                            Msg::Hand(b) => batch.push(*b),
                            Msg::Flush(f) => flushes.push(f),
                        }
                        if batch.len() >= 500 || !flushes.is_empty() {
                            break;
                        }
                        match rx.try_recv() {
                            Ok(m) => next = Some(m),
                            Err(TryRecvError::Empty) => {}
                            Err(TryRecvError::Disconnected) => {}
                        }
                    }
                    if !batch.is_empty() {
                        if let Err(e) = store.append_hands(&batch) {
                            tracing::error!("hand writer: {e}");
                        }
                        batch.clear();
                    }
                    for f in flushes.drain(..) {
                        let _ = f.send(());
                    }
                }
            })
            .expect("spawn writer thread");
        HandWriter { tx }
    }

    pub fn push(&self, h: HandHistory, events: Vec<Value>) {
        let _ = self.tx.send(Msg::Hand(Box::new((h, events))));
    }

    /// Resolves once everything pushed before this call is stored.
    pub async fn flush(&self) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if self.tx.send(Msg::Flush(tx)).is_ok() {
            let _ = rx.await;
        }
    }
}
