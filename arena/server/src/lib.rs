//! Poker Sharingan arena table server. See README.md and docs/TABLE_API.md.

pub mod api;
pub mod config;
pub mod deck;
pub mod error;
pub mod hand;
pub mod house;
pub mod hub;
pub mod matches;
pub mod model;
pub mod state;
pub mod stats;
pub mod store;
pub mod table;
pub mod util;
pub mod benchmarks;
pub mod webhook;
pub mod writer;
pub mod ws;

use config::ServerConfig;
use state::AppState;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

pub fn build_state(cfg: ServerConfig) -> Result<Arc<AppState>, String> {
    let store = Arc::new(store::SqliteStore::open(&cfg.db_path).map_err(|e| e.to_string())?);
    let house = house::HouseRegistry::build(&cfg)?;
    house.register_players(store.as_ref()).map_err(|e| e.to_string())?;
    let n = store::Store::abort_stale_matches(store.as_ref()).map_err(|e| e.to_string())?;
    if n > 0 {
        tracing::warn!("marked {n} stale running matches as aborted");
    }
    let admin_token = cfg.admin_token.clone().unwrap_or_else(|| format!("adm_{}", util::random_hex()));
    Ok(Arc::new(AppState {
        cfg,
        writer: writer::HandWriter::spawn(store.clone()),
        store,
        hub: Arc::new(hub::Hub::default()),
        house: Arc::new(house),
        admin_token,
        tables: RwLock::new(HashMap::new()),
        matches: RwLock::new(HashMap::new()),
    }))
}

pub struct RunningServer {
    pub addr: SocketAddr,
    pub app: Arc<AppState>,
    pub task: tokio::task::JoinHandle<()>,
}

/// Bind and serve in the background (used by `main` and by the integration tests).
pub async fn start(cfg: ServerConfig) -> Result<RunningServer, String> {
    let bind = cfg.bind.clone();
    let app = build_state(cfg)?;
    let listener = tokio::net::TcpListener::bind(&bind).await.map_err(|e| format!("bind {bind}: {e}"))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    // Small frames per decision: Nagle + delayed ACK would add ~40 ms to every round trip.
    use axum::serve::ListenerExt;
    let listener = listener.tap_io(|tcp| {
        let _ = tcp.set_nodelay(true);
    });
    let router = api::router(app.clone());
    let task = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router).await {
            tracing::error!("server error: {e}");
        }
    });
    Ok(RunningServer { addr, app, task })
}
