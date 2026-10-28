//! Shared application state.

use crate::config::ServerConfig;
use crate::error::{ApiError, Res};
use crate::house::HouseRegistry;
use crate::hub::Hub;
use crate::matches::MatchRuntime;
use crate::model::PlayerRecord;
use crate::store::Store;
use crate::table::TableShared;
use crate::writer::HandWriter;
use crate::util::{ct_eq, sha256_hex};
use axum::http::HeaderMap;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

pub struct AppState {
    pub cfg: ServerConfig,
    pub store: Arc<dyn Store>,
    pub hub: Arc<Hub>,
    pub house: Arc<HouseRegistry>,
    pub writer: HandWriter,
    pub admin_token: String,
    pub limiter: crate::limits::RateLimiter,
    /// Set when shutdown starts: readiness fails and new tables/matches/runs/sockets are refused.
    pub draining: AtomicBool,
    /// `true` once sockets must close (last phase of shutdown).
    pub shutdown: tokio::sync::watch::Sender<bool>,
    pub tables: RwLock<HashMap<String, Arc<TableShared>>>,
    pub matches: RwLock<HashMap<String, Arc<MatchRuntime>>>,
}

#[derive(Clone, Debug)]
pub enum Principal {
    Admin,
    /// The trusted eval runner (`ARENA_RUNNER_TOKEN`); only valid on the hosted-attestation / run endpoints.
    Runner,
    Player(PlayerRecord),
}

impl Principal {
    pub fn is_admin(&self) -> bool {
        matches!(self, Principal::Admin)
    }
    /// Id used as table/match owner.
    pub fn owner_id(&self) -> String {
        match self {
            Principal::Admin => "admin".into(),
            Principal::Runner => "runner".into(),
            Principal::Player(p) => p.player_id.clone(),
        }
    }
}

pub fn bearer(headers: &HeaderMap) -> Option<String> {
    let v = headers.get("authorization")?.to_str().ok()?;
    let t = v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer "))?;
    Some(t.trim().to_string())
}

impl AppState {
    pub fn authenticate_token(&self, token: &str) -> Res<Principal> {
        if ct_eq(token, &self.admin_token) {
            return Ok(Principal::Admin);
        }
        if let Some(rt) = &self.cfg.runner_token {
            if ct_eq(token, rt) {
                return Ok(Principal::Runner);
            }
        }
        match self.store.player_by_token_hash(&sha256_hex(token.as_bytes()))? {
            Some(p) => Ok(Principal::Player(p)),
            None => Err(ApiError::unauthorized()),
        }
    }
    /// Admin or bot token. The runner token is refused (403): it may only call the run/attestation endpoints.
    pub fn authenticate(&self, headers: &HeaderMap) -> Res<Principal> {
        match self.authenticate_any(headers)? {
            Principal::Runner => Err(ApiError::forbidden("the runner token is only valid for benchmark runs and /v1/runner/*")),
            p => Ok(p),
        }
    }
    /// Like `authenticate`, but also accepts the runner token.
    pub fn authenticate_any(&self, headers: &HeaderMap) -> Res<Principal> {
        let t = bearer(headers).ok_or_else(ApiError::unauthorized)?;
        self.authenticate_token(&t)
    }
    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::SeqCst)
    }
    /// 503 while shutting down; call at the top of every handler that starts new work.
    pub fn ensure_accepting(&self) -> Res<()> {
        if self.is_draining() {
            Err(ApiError::shutting_down())
        } else {
            Ok(())
        }
    }
    /// Free (non-match) tables that are open and that the player owns or sits at.
    pub fn open_tables_of(&self, player_id: &str) -> usize {
        self.tables
            .read()
            .unwrap()
            .values()
            .filter(|t| {
                let i = t.inner.lock().unwrap();
                i.match_id.is_none()
                    && i.status != "closed"
                    && (i.owner == player_id || i.seats.iter().any(|s| s.player_id.as_deref() == Some(player_id)))
            })
            .count()
    }
    /// Matches (including benchmark runs) owned by `owner` that have not ended.
    pub fn live_matches_of(&self, owner: &str) -> usize {
        self.matches
            .read()
            .unwrap()
            .values()
            .filter(|m| {
                let s = m.state.lock().unwrap();
                !matches!(s.rec.status.as_str(), "finished" | "aborted")
                    && self.tables.read().unwrap().get(&m.table_id).map(|t| t.inner.lock().unwrap().owner == owner).unwrap_or(false)
            })
            .count()
    }
    pub fn table(&self, id: &str) -> Res<Arc<TableShared>> {
        self.tables.read().unwrap().get(id).cloned().ok_or_else(|| ApiError::not_found("table"))
    }
    pub fn match_rt(&self, id: &str) -> Res<Arc<MatchRuntime>> {
        self.matches.read().unwrap().get(id).cloned().ok_or_else(|| ApiError::not_found("match"))
    }
    /// Tables where the player holds a seat.
    pub fn tables_of(&self, player_id: &str) -> Vec<Arc<TableShared>> {
        self.tables
            .read()
            .unwrap()
            .values()
            .filter(|t| t.inner.lock().unwrap().seats.iter().any(|s| s.player_id.as_deref() == Some(player_id)))
            .cloned()
            .collect()
    }
}
