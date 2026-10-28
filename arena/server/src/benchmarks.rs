//! Benchmark tracks: versioned, frozen opponent panels. A run is a duplicate match of one bot
//! against the track's panel with a fixed seed (every bot sees the same deals, so scores are paired).
//!
//! Verification: a run is `verified` only if the server itself ran the bot (house/hosted bots).
//! A remote bot's run is stored and ranked but flagged unverified, because the bot code runs
//! on the owner's machine and the seed is public.

use crate::error::{ApiError, Res};
use crate::house::house_id;
use crate::matches::{self, BenchLink, NewMatch};
use crate::model::*;
use crate::state::AppState;
use crate::util::*;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct Track {
    pub id: &'static str,
    pub version: &'static str,
    pub description: &'static str,
    /// The five opponents, house bot names.
    pub panel: &'static [&'static str],
}

pub const TRACKS: &[Track] = &[
    Track {
        id: "ladder-v1",
        version: "1",
        description: "Rule bots weak to strong, then SHARINGAN-1: random, calling_station, 2x tight_aggressive, sharingan-1.",
        panel: &["random", "calling_station", "tight_aggressive", "tight_aggressive", "sharingan-1"],
    },
    Track {
        id: "sharingan-1",
        version: "1",
        description: "Five copies of SHARINGAN-1.",
        panel: &["sharingan-1", "sharingan-1", "sharingan-1", "sharingan-1", "sharingan-1"],
    },
];

/// Server-side default size of a benchmark run (hands).
pub const DEFAULT_HANDS: u32 = 20_000;

pub fn track(id: &str) -> Res<&'static Track> {
    TRACKS.iter().find(|t| t.id == id).ok_or_else(|| ApiError::not_found("benchmark track"))
}

fn track_json(app: &AppState, t: &Track) -> Value {
    let available = app.house.bots.contains_key("sharingan-1");
    json!({"track": t.id, "version": t.version, "description": t.description,
           "panel": t.panel.iter().map(|k| format!("house:{k}")).collect::<Vec<_>>(),
           "seed": seed_of(t), "default_hands": DEFAULT_HANDS, "available": available,
           "blueprint_fingerprint": app.house.blueprint_fp})
}

fn seed_of(t: &Track) -> String {
    format!("bench:{}:v{}", t.id, t.version)
}

type App = State<Arc<AppState>>;

pub async fn list_tracks(State(app): App) -> Json<Value> {
    Json(json!(TRACKS.iter().map(|t| track_json(&app, t)).collect::<Vec<_>>()))
}

pub async fn get_track(State(app): App, Path(id): Path<String>) -> Res<Json<Value>> {
    let t = track(&id)?;
    let mut v = track_json(&app, t);
    let mut runs: Vec<RunRecord> = app.store.list_runs(Some(t.id), None, 10_000)?.into_iter().filter(|r| r.status == "finished").collect();
    runs.sort_by(|a, b| b.mbb_per_game.partial_cmp(&a.mbb_per_game).unwrap_or(std::cmp::Ordering::Equal));
    v["top_runs"] = json!(runs.into_iter().take(50).collect::<Vec<_>>());
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct RunReq {
    #[serde(default)]
    player_id: Option<String>,
    #[serde(default)]
    hands: Option<u32>,
    /// Parallel tables (default: 1 for remote bots, up to 8 for house bots).
    #[serde(default)]
    lanes: Option<usize>,
    #[serde(default)]
    decision_time_ms: Option<u64>,
    /// Runner token only: the trusted runner is playing this bot on operator-controlled infrastructure,
    /// so the run is `verified`. Anyone else sending `hosted: true` gets 403.
    #[serde(default)]
    hosted: Option<bool>,
}

pub async fn create_run(
    State(app): App,
    headers: HeaderMap,
    Path(tid): Path<String>,
    body: Option<Json<RunReq>>,
) -> Res<(StatusCode, Json<Value>)> {
    app.ensure_accepting()?;
    let who = app.authenticate_any(&headers)?;
    let t = track(&tid)?;
    let req = body.map(|b| b.0).unwrap_or(RunReq { player_id: None, hands: None, lanes: None, decision_time_ms: None, hosted: None });
    let runner = matches!(who, crate::state::Principal::Runner);
    if req.hosted == Some(true) && !runner {
        return Err(ApiError::forbidden("only the runner token (ARENA_RUNNER_TOKEN) can start a hosted (verified) run"));
    }
    crate::api::check_match_quota(&app, &who)?;
    if matches!(who, crate::state::Principal::Player(_)) && req.hands.map(|h| h > app.cfg.max_run_hands).unwrap_or(false) {
        return Err(ApiError::bad_request(format!("hands exceeds the per-run limit of {}", app.cfg.max_run_hands))
            .with(json!({"limit": "ARENA_MAX_RUN_HANDS", "max": app.cfg.max_run_hands})));
    }
    let pid = match (&req.player_id, &who) {
        (Some(p), _) if crate::house::HouseRegistry::looks_like_house(p) => house_id(&app.house.resolve(p)?),
        (Some(p), _) => p.clone(),
        (None, crate::state::Principal::Player(p)) => p.player_id.clone(),
        (None, _) => return Err(ApiError::bad_request("player_id required")),
    };
    let rec = app.store.get_player(&pid)?.ok_or_else(|| ApiError::not_found("player"))?;
    if !who.is_admin() && !runner && who.owner_id() != rec.player_id {
        return Err(ApiError::forbidden("you can only run your own bot (admins can run any, including house bots)"));
    }
    let mut panel = Vec::new();
    for k in t.panel {
        panel.push(house_id(&app.house.resolve(k)?));
    }
    if !rec.house && !app.hub.connected(&rec.player_id) {
        return Err(ApiError::conflict("not_connected", "connect to /v1/play with your bot token before starting a run"));
    }
    let hosted = runner && req.hosted == Some(true);
    let hands = req.hands.unwrap_or(DEFAULT_HANDS).clamp(6, 1_000_000);
    let hands = (hands + 5) / 6 * 6;
    let lanes = req.lanes.unwrap_or(if rec.house { std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8) } else { 1 });
    let run_id = new_id("r");
    let seed = seed_of(t);
    let mut participants = vec![rec.player_id.clone()];
    participants.extend(panel.iter().cloned());
    let cfg = TableConfig {
        mode: Some("benchmark".into()),
        decision_time_ms: req.decision_time_ms.unwrap_or(5000),
        min_players_to_deal: 6,
        ..Default::default()
    };
    cfg.validate()?;
    let rt = matches::create(
        &app,
        NewMatch {
            owner: who.owner_id(),
            cfg,
            participants,
            hands,
            duplicate: Some(DuplicateConfig { enabled: true, rotations: 6, deck_from: "seed".into() }),
            seed: seed.clone(),
            aivat: "full".into(),
            block_deals: 12,
            hide_cards: true,
            lanes,
            bench: Some(BenchLink { run_id: run_id.clone(), track: t.id.into(), player_id: rec.player_id.clone() }),
        },
    )?;
    let run = RunRecord {
        run_id,
        track: t.id.into(),
        panel_version: t.version.into(),
        panel: panel.clone(),
        player_id: rec.player_id.clone(),
        name: rec.name.clone(),
        match_id: rt.id.clone(),
        seed,
        hands_planned: hands,
        hands_done: 0,
        status: "running".into(),
        verified: rec.house || hosted,
        hosted,
        mbb_per_game: None,
        ci95_mbb: None,
        method: None,
        blueprint_fingerprint: app.house.blueprint_fp.clone(),
        created_at: now_iso(),
        finished_at: None,
    };
    app.store.put_run(&run)?;
    matches::start(&app, &rt)?;
    Ok((StatusCode::CREATED, Json(run_json(&app, run))))
}

/// A run with live progress while it is playing: `hands_done`, `hands_total`, running estimate.
fn run_json(app: &AppState, run: RunRecord) -> Value {
    let mut v = serde_json::to_value(&run).unwrap();
    v["hands_total"] = json!(run.hands_planned);
    if run.status == "running" {
        if let Ok(rt) = app.match_rt(&run.match_id) {
            if let Some(p) = rt.progress() {
                v["hands_done"] = p["hands_done"].clone();
                v["running_mbb_per_game"] = p["mbb_per_game"].clone();
                v["running_ci95_mbb"] = p["ci95_mbb"].clone();
                v["running_method"] = p["method"].clone();
            }
        }
    }
    v
}

fn find_run(app: &AppState, tid: &str, rid: &str) -> Res<RunRecord> {
    let r = app.store.get_run(rid)?.ok_or_else(|| ApiError::not_found("run"))?;
    if r.track != tid {
        return Err(ApiError::not_found("run"));
    }
    Ok(r)
}

pub async fn get_run(State(app): App, Path((tid, rid)): Path<(String, String)>) -> Res<Json<Value>> {
    track(&tid)?;
    let r = find_run(&app, &tid, &rid)?;
    Ok(Json(run_json(&app, r)))
}

#[derive(Deserialize)]
pub struct RunsQ {
    #[serde(default)]
    player_id: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

pub async fn list_runs(State(app): App, Path(tid): Path<String>, Query(q): Query<RunsQ>) -> Res<Json<Value>> {
    track(&tid)?;
    let runs = app.store.list_runs(Some(&tid), q.player_id.as_deref(), q.limit.unwrap_or(100).min(1000))?;
    Ok(Json(json!(runs.into_iter().map(|r| run_json(&app, r)).collect::<Vec<_>>())))
}

pub async fn abort_run(State(app): App, headers: HeaderMap, Path((tid, rid)): Path<(String, String)>) -> Res<Json<Value>> {
    let who = app.authenticate_any(&headers)?;
    let r = find_run(&app, &tid, &rid)?;
    if !who.is_admin() && !matches!(who, crate::state::Principal::Runner) && who.owner_id() != r.player_id {
        return Err(ApiError::forbidden("not your run"));
    }
    let rt = app.match_rt(&r.match_id)?;
    {
        let mut s = rt.state.lock().unwrap();
        if s.rec.status == "finished" || s.rec.status == "aborted" {
            return Err(ApiError::conflict("match_not_running", format!("run is {}", s.rec.status)));
        }
        s.abort = true;
        s.rec.status = "aborted".into();
    }
    matches::wake_all(&app, &rt);
    Ok(Json(run_json(&app, r)))
}
