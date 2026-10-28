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
use std::sync::{Arc, RwLock};

pub struct AppState {
    pub cfg: ServerConfig,
    pub store: Arc<dyn Store>,
    pub hub: Arc<Hub>,
    pub house: Arc<HouseRegistry>,
    pub writer: HandWriter,
    pub admin_token: String,
    pub tables: RwLock<HashMap<String, Arc<TableShared>>>,
    pub matches: RwLock<HashMap<String, Arc<MatchRuntime>>>,
}

#[derive(Clone, Debug)]
pub enum Principal {
    Admin,
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
        match self.store.player_by_token_hash(&sha256_hex(token.as_bytes()))? {
            Some(p) => Ok(Principal::Player(p)),
            None => Err(ApiError::unauthorized()),
        }
    }
    pub fn authenticate(&self, headers: &HeaderMap) -> Res<Principal> {
        let t = bearer(headers).ok_or_else(ApiError::unauthorized)?;
        self.authenticate_token(&t)
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
