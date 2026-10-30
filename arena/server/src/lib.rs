//! Poker Sharingan arena table server. See README.md and docs/TABLE_API.md.

pub mod api;
pub mod config;
pub mod deck;
pub mod error;
pub mod hand;
pub mod house;
pub mod hub;
pub mod limits;
pub mod matches;
pub mod model;
pub mod pgstore;
pub mod state;
pub mod stats;
pub mod store;
pub mod table;
pub mod util;
pub mod benchmarks;
pub mod webhook;
pub mod writer;
pub mod worker;
pub mod pipe_bot;
pub mod results;
pub mod ws;

use config::ServerConfig;
use state::AppState;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

pub fn build_state(cfg: ServerConfig) -> Result<Arc<AppState>, String> {
    let store: Arc<dyn store::Store> = match &cfg.database_url {
        Some(url) => Arc::new(pgstore::PgStore::open(url, cfg.database_schema.as_deref(), cfg.db_pool)?),
        None => Arc::new(store::SqliteStore::open(&cfg.db_path).map_err(|e| e.to_string())?),
    };
    let house = house::HouseRegistry::build(&cfg)?;
    house.register_players(store.as_ref()).map_err(|e| e.to_string())?;
    let n = store.abort_stale_matches().map_err(|e| e.to_string())?;
    if n > 0 {
        tracing::warn!("marked {n} stale running matches as aborted");
    }
    let admin_token = cfg.admin_token.clone().unwrap_or_else(|| format!("adm_{}", util::random_hex()));
    let (rate_rps, rate_burst, bots_per_day) = (cfg.rate_limit_rps, cfg.rate_limit_burst, cfg.bots_per_ip_per_day);
    Ok(Arc::new(AppState {
        cfg,
        writer: writer::HandWriter::spawn(store.clone()),
        store,
        hub: Arc::new(hub::Hub::default()),
        house: Arc::new(house),
        admin_token,
        limiter: limits::RateLimiter::new(rate_rps, rate_burst),
        reg_limiter: limits::RateLimiter::new(bots_per_day / 86_400.0, bots_per_day),
        draining: std::sync::atomic::AtomicBool::new(false),
        shutdown: tokio::sync::watch::channel(false).0,
        tables: RwLock::new(HashMap::new()),
        matches: RwLock::new(HashMap::new()),
    }))
}

pub struct RunningServer {
    pub addr: SocketAddr,
    pub app: Arc<AppState>,
    pub task: tokio::task::JoinHandle<()>,
    stop: tokio::sync::watch::Sender<bool>,
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
    let (stop, mut stop_rx) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(async move {
        let svc = router.into_make_service_with_connect_info::<SocketAddr>();
        let r = axum::serve(listener, svc)
            .with_graceful_shutdown(async move {
                let _ = stop_rx.wait_for(|v| *v).await;
            })
            .await;
        if let Err(e) = r {
            tracing::error!("server error: {e}");
        }
    });
    Ok(RunningServer { addr, app, task, stop })
}

impl RunningServer {
    /// Graceful shutdown, in phases:
    /// 1. drain: `/readyz` turns 503 and new tables, matches, runs and sockets are refused (503 `shutting_down`);
    /// 2. wait up to `ARENA_SHUTDOWN_GRACE_MS` for running matches to finish (REST stays up so clients can read results);
    /// 3. abort what is left (partial results are kept), flush hand histories to the database;
    /// 4. close tables and sockets (close frame), stop accepting connections, wait for in-flight requests.
    pub async fn shutdown(self) {
        let app = self.app.clone();
        app.draining.store(true, std::sync::atomic::Ordering::SeqCst);
        let grace = std::time::Duration::from_millis(app.cfg.shutdown_grace_ms);
        let running = || -> Vec<Arc<matches::MatchRuntime>> {
            app.matches
                .read()
                .unwrap()
                .values()
                .filter(|m| m.state.lock().unwrap().rec.status == "running")
                .cloned()
                .collect()
        };
        let t0 = std::time::Instant::now();
        while !running().is_empty() && t0.elapsed() < grace {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let left = running();
        if !left.is_empty() {
            tracing::warn!("shutdown: aborting {} unfinished matches", left.len());
            for rt in &left {
                {
                    let mut s = rt.state.lock().unwrap();
                    s.abort = true;
                }
                matches::wake_all(&app, rt);
            }
            let t1 = std::time::Instant::now();
            while !running().is_empty() && t1.elapsed() < std::time::Duration::from_secs(10) {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
        app.writer.flush().await;
        let tables: Vec<_> = app.tables.read().unwrap().values().cloned().collect();
        for t in tables {
            let _ = t.tx.send(table::TableCmd::Close);
            t.wake.notify_one();
        }
        let _ = app.shutdown.send(true);
        let _ = self.stop.send(true);
        let mut task = self.task;
        if tokio::time::timeout(std::time::Duration::from_secs(5), &mut task).await.is_err() {
            task.abort();
        }
        app.writer.flush().await;
        tracing::info!("shutdown complete");
    }
}
