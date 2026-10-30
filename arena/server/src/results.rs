//! Control-plane intake of match-worker results: `POST /v1/results` (TABLE_API.md 14.8).
//!
//! A worker (`sharingan-arena run --sign-key-file`) plays a benchmark on operator-controlled
//! infrastructure and signs its `result.json` with HMAC-SHA256. The runner/admin posts it here; if the
//! signature matches one of `ARENA_WORKER_KEYS` and the result is for the track's frozen panel and seed,
//! it is stored as a finished benchmark run with `verified: true, hosted: true`
//! (`deals: "frozen"` on the track seed, `"fresh"` on a per-run `run:<hex>` seed), which the leaderboard
//! picks up like any other run.

use crate::benchmarks;
use crate::error::{ApiError, Res};
use crate::house::{house_id, HouseRegistry};
use crate::model::RunRecord;
use crate::state::{AppState, Principal};
use crate::util::*;
use crate::worker::{verify_result, RESULT_SCHEMA};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde_json::{json, Value};
use std::sync::Arc;

pub const MIN_RUN_SEED_HEX: usize = 32;
pub const MAX_RUN_SEED_HEX: usize = 128;

/// A per-run secret seed of a hosted run: `run:` + 32..128 hex characters. The runner draws it fresh for every job and
/// keeps it from the bot until the run is over, so the deals cannot be known (or tabulated) in advance.
pub fn is_run_seed(s: &str) -> bool {
    match s.strip_prefix("run:") {
        Some(h) => (MIN_RUN_SEED_HEX..=MAX_RUN_SEED_HEX).contains(&h.len()) && h.bytes().all(|b| b.is_ascii_hexdigit()),
        None => false,
    }
}

fn bad(m: impl Into<String>) -> ApiError {
    ApiError::bad_request(m)
}

pub async fn post_result(State(app): State<Arc<AppState>>, headers: HeaderMap, Json(body): Json<Value>) -> Res<(StatusCode, Json<Value>)> {
    app.ensure_accepting()?;
    match app.authenticate_any(&headers)? {
        Principal::Admin | Principal::Runner => {}
        Principal::Player(_) => return Err(ApiError::forbidden("only the runner token (ARENA_RUNNER_TOKEN) or an admin can post worker results")),
    }
    // Either the raw result.json, or {"result": {...}, "player_id": "..."}.
    let (result, wrapper_pid) = match (body.get("result"), body.get("schema")) {
        (Some(r), None) if r.is_object() => (r.clone(), body.get("player_id").and_then(|p| p.as_str()).map(String::from)),
        _ => (body.clone(), None),
    };
    let key_id = verify_result(&result, &app.cfg.worker_keys).map_err(|m| ApiError::new(401, "invalid_signature", m))?;

    if result["schema"] != RESULT_SCHEMA {
        return Err(bad(format!("schema must be {RESULT_SCHEMA}")));
    }
    let t = benchmarks::track(result["track"].as_str().unwrap_or(""))?;
    if result["panel_version"].as_str() != Some(t.version) {
        return Err(bad(format!("panel_version must be {} (the current version of {})", t.version, t.id)));
    }
    let panel: Vec<String> = t.panel.iter().map(|k| house_id(k)).collect();
    if result["panel"] != json!(panel) {
        return Err(bad("panel is not the track's frozen panel"));
    }
    let track_seed = benchmarks::seed_of(t);
    let seed = result["seed"].as_str().unwrap_or("").to_string();
    let deals = if seed == track_seed {
        "frozen"
    } else if is_run_seed(&seed) {
        "fresh"
    } else {
        return Err(bad(format!(
            "seed must be the track seed {track_seed} (frozen deals) or a per-run seed run:<{MIN_RUN_SEED_HEX}-{MAX_RUN_SEED_HEX} hex characters> (fresh secret deals)"
        )));
    };
    let run_id = result["run_id"].as_str().unwrap_or("");
    if !(8..=64).contains(&run_id.len()) || !run_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(bad("run_id must be 8-64 characters of [A-Za-z0-9_-]"));
    }
    if app.store.get_run(run_id)?.is_some() {
        return Err(ApiError::conflict("duplicate_result", "a result with this run_id was already accepted"));
    }
    let pid_raw = match (wrapper_pid.as_deref(), result["player_id"].as_str()) {
        (Some(a), Some(b)) if a != b => return Err(bad("player_id differs from the signed player_id")),
        (Some(a), _) | (None, Some(a)) => a.to_string(),
        (None, None) => return Err(bad("player_id required (signed in the result via --player-id, or wrapper field)")),
    };
    let pid = if HouseRegistry::looks_like_house(&pid_raw) { house_id(&app.house.resolve(&pid_raw)?) } else { pid_raw };
    let player = app.store.get_player(&pid)?.ok_or_else(|| ApiError::not_found("player"))?;
    let (hands, planned) = (result["hands"].as_u64().unwrap_or(0) as u32, result["hands_planned"].as_u64().unwrap_or(0) as u32);
    if hands < 6 || hands != planned {
        return Err(bad("hands must equal hands_planned (>= 6): incomplete results are not accepted"));
    }
    let (mbb, ci) = (result["mbb_per_game"].as_f64(), result["ci95_mbb"].as_f64());
    let (Some(mbb), Some(ci)) = (mbb, ci) else { return Err(bad("mbb_per_game and ci95_mbb must be numbers")) };
    if !mbb.is_finite() || !ci.is_finite() {
        return Err(bad("mbb_per_game and ci95_mbb must be finite"));
    }
    let fp = result["blueprint_fingerprint"].as_str().map(String::from);
    if let Some(server_fp) = &app.house.blueprint_fp {
        if fp.as_ref() != Some(server_fp) {
            return Err(ApiError::conflict("blueprint_mismatch", "the worker used a different blueprint than this arena's SHARINGAN-1")
                .with(json!({"server": server_fp, "result": fp})));
        }
    }
    let run = RunRecord {
        run_id: run_id.to_string(),
        track: t.id.into(),
        panel_version: t.version.into(),
        panel,
        player_id: player.player_id.clone(),
        name: player.name.clone(),
        match_id: format!("worker:{run_id}"),
        seed,
        deals: Some(deals.into()),
        hands_planned: planned,
        hands_done: hands,
        status: "finished".into(),
        verified: true,
        hosted: true,
        mbb_per_game: Some(mbb),
        ci95_mbb: Some(ci),
        method: result["method"].as_str().map(String::from),
        blueprint_fingerprint: fp,
        created_at: now_iso(),
        finished_at: Some(result["finished_at"].as_str().map(String::from).unwrap_or_else(now_iso)),
    };
    app.store.put_run(&run)?;
    let mut v = serde_json::to_value(&run).unwrap();
    v["hands_total"] = json!(run.hands_planned);
    v["key_id"] = json!(key_id);
    v["forfeit_hands"] = result["forfeit_hands"].clone();
    Ok((StatusCode::CREATED, Json(v)))
}
