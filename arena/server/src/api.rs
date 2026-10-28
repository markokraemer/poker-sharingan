//! REST surface (TABLE_API.md section 3).

use crate::error::{ApiError, Res};
use crate::house::{house_id, HouseRegistry};
use crate::matches::{self, MatchRuntime, NewMatch};
use crate::model::*;
use crate::state::{AppState, Principal};
use crate::stats::leaderboard;
use crate::table::{spawn_table, SeatState};
use crate::util::*;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;

type App = State<Arc<AppState>>;

pub fn router(app: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/livez", get(|| async { "ok" }))
        .route("/readyz", get(readyz))
        .route("/v1/runner/hosted", post(runner_hosted))
        .route("/v1/results", post(crate::results::post_result))
        .route("/v1/bots", post(register).get(list_players))
        .route("/v1/players", post(register).get(list_players))
        .route("/v1/bots/{id}", get(get_player))
        .route("/v1/players/{id}", get(get_player))
        .route("/v1/house", get(list_house))
        .route("/v1/tables", post(create_table).get(list_tables))
        .route("/v1/tables/{id}", get(get_table).delete(close_table))
        .route("/v1/tables/{id}/seats/{seat}", post(sit).delete(stand))
        .route("/v1/tables/{id}/fill", post(fill))
        .route("/v1/matches", post(create_match).get(list_matches))
        .route("/v1/matches/{id}", get(get_match))
        .route("/v1/matches/{id}/start", post(start_match))
        .route("/v1/matches/{id}/resume", post(start_match))
        .route("/v1/matches/{id}/pause", post(pause_match))
        .route("/v1/matches/{id}/abort", post(abort_match))
        .route("/v1/matches/{id}/hands", get(match_hands))
        .route("/v1/matches/{id}/hands.jsonl", get(match_hands_jsonl))
        .route("/v1/hands/{id}", get(get_hand))
        .route("/v1/hands/{id}/events", get(get_hand_events))
        .route("/v1/leaderboard", get(get_leaderboard))
        .route("/v1/benchmarks", get(crate::benchmarks::list_tracks))
        .route("/v1/benchmarks/{track}", get(crate::benchmarks::get_track))
        .route("/v1/benchmarks/{track}/runs", post(crate::benchmarks::create_run).get(crate::benchmarks::list_runs))
        .route("/v1/benchmarks/{track}/runs/{run}", get(crate::benchmarks::get_run))
        .route("/v1/benchmarks/{track}/runs/{run}/abort", post(crate::benchmarks::abort_run))
        .route("/v1/play", get(crate::ws::play))
        .layer(axum::extract::DefaultBodyLimit::max(app.cfg.max_body_bytes))
        .layer(axum::middleware::from_fn_with_state(app.clone(), json_413))
        .layer(axum::middleware::from_fn_with_state(app.clone(), rate_limit))
        .layer(axum::middleware::from_fn(cors))
        .with_state(app)
}

/// Per-token (anonymous: per-IP) token-bucket limiter; 429 `rate_limited` with `Retry-After`.
/// Health probes, CORS preflights and the admin/runner tokens are exempt.
async fn rate_limit(State(app): App, req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let path = req.uri().path();
    if !app.limiter.enabled() || matches!(path, "/healthz" | "/livez" | "/readyz") || req.method() == axum::http::Method::OPTIONS {
        return next.run(req).await;
    }
    let tok = crate::state::bearer(req.headers());
    if let Some(t) = &tok {
        if ct_eq(t, &app.admin_token) || app.cfg.runner_token.as_deref().map(|r| ct_eq(t, r)).unwrap_or(false) {
            return next.run(req).await;
        }
    }
    let key = match &tok {
        Some(t) => format!("t:{}", &sha256_hex(t.as_bytes())[..16]),
        None => {
            let fwd = if app.cfg.trust_proxy {
                req.headers().get("x-forwarded-for").and_then(|v| v.to_str().ok()).and_then(|v| v.split(',').next()).map(|s| s.trim().to_string())
            } else {
                None
            };
            let ip = fwd.or_else(|| {
                req.extensions().get::<axum::extract::ConnectInfo<std::net::SocketAddr>>().map(|c| c.0.ip().to_string())
            });
            format!("ip:{}", ip.unwrap_or_default())
        }
    };
    match app.limiter.check(&key) {
        Ok(()) => next.run(req).await,
        Err(ms) => ApiError::rate_limited(
            "too many requests; slow down",
            json!({"limit": "ARENA_RATE_LIMIT_RPS", "rps": app.cfg.rate_limit_rps, "burst": app.cfg.rate_limit_burst, "retry_after_ms": ms}),
        )
        .into_response(),
    }
}

/// Body-limit rejections come back as plain text; give them the contract's error format.
async fn json_413(State(app): App, req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let res = next.run(req).await;
    if res.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return ApiError::payload_too_large(app.cfg.max_body_bytes).into_response();
    }
    res
}

/// Load-balancer readiness: 200 only when the database answers and the server is not draining.
async fn readyz(State(app): App) -> Response {
    let a = app.clone();
    let db_ok = tokio::task::spawn_blocking(move || a.store.ping().is_ok()).await.unwrap_or(false);
    let draining = app.is_draining();
    let running = app.matches.read().unwrap().values().filter(|m| m.state.lock().unwrap().rec.status == "running").count();
    let status = if draining { "draining" } else if !db_ok { "unavailable" } else { "ready" };
    let body = json!({"status": status, "db": if db_ok { "ok" } else { "error" }, "draining": draining,
        "tables": app.tables.read().unwrap().len(), "matches_running": running, "ws_connections": app.hub.total()});
    ((if status == "ready" { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE }), Json(body)).into_response()
}

#[derive(Deserialize)]
struct HostedReq {
    player_id: String,
    run_id: String,
    #[serde(default = "yes")]
    hosted: bool,
}
fn yes() -> bool {
    true
}

/// Runner-only: register (or revoke) a bot as `hosted` for one running benchmark run. Only a hosted
/// run is `verified`; the flag is stored with the run and shows on the leaderboard.
async fn runner_hosted(State(app): App, headers: HeaderMap, Json(req): Json<HostedReq>) -> Res<Json<Value>> {
    match app.authenticate_any(&headers)? {
        Principal::Runner => {}
        _ => return Err(ApiError::forbidden("only the runner token (ARENA_RUNNER_TOKEN) can attest hosted bots")),
    }
    let mut run = app.store.get_run(&req.run_id)?.ok_or_else(|| ApiError::not_found("run"))?;
    let pid = if HouseRegistry::looks_like_house(&req.player_id) { house_id(&app.house.resolve(&req.player_id)?) } else { req.player_id.clone() };
    if run.player_id != pid {
        return Err(ApiError::bad_request("run_id does not belong to that player_id"));
    }
    if run.status != "running" {
        return Err(ApiError::conflict("match_not_running", format!("run is {}; attest while it is running", run.status)));
    }
    let house = app.store.get_player(&pid)?.map(|p| p.house).unwrap_or(false);
    run.hosted = req.hosted;
    run.verified = req.hosted || house;
    app.store.put_run(&run)?;
    Ok(Json(json!({"run_id": run.run_id, "player_id": run.player_id, "hosted": run.hosted, "verified": run.verified})))
}

async fn close_table(State(app): App, headers: HeaderMap, Path(id): Path<String>) -> Res<Json<Value>> {
    let who = app.authenticate(&headers)?;
    let t = app.table(&id)?;
    let (owner, is_match) = {
        let i = t.inner.lock().unwrap();
        (i.owner.clone(), i.match_id.is_some())
    };
    if !who.is_admin() && who.owner_id() != owner {
        return Err(ApiError::forbidden("only the table owner or an admin can close a table"));
    }
    if is_match {
        return Err(ApiError::conflict("table_locked", "match tables close with their match (abort the match)"));
    }
    let _ = t.tx.send(crate::table::TableCmd::Close);
    t.wake.notify_one();
    t.inner.lock().unwrap().status = "closed".into();
    Ok(Json(t.table_json(&app.hub)))
}

/// Minimal CORS so the static site (a different origin) can call the REST API from a browser.
/// Auth is a bearer token (no cookies), so `*` is safe.
async fn cors(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let preflight = req.method() == axum::http::Method::OPTIONS;
    let mut res = if preflight { StatusCode::NO_CONTENT.into_response() } else { next.run(req).await };
    let h = res.headers_mut();
    h.insert("access-control-allow-origin", "*".parse().unwrap());
    h.insert("access-control-allow-methods", "GET, POST, DELETE, OPTIONS".parse().unwrap());
    h.insert("access-control-allow-headers", "authorization, content-type".parse().unwrap());
    h.insert("access-control-max-age", "86400".parse().unwrap());
    res
}

fn public_player(p: &PlayerRecord) -> Value {
    json!({"player_id": p.player_id, "bot_id": p.player_id, "name": p.name, "kind": p.kind,
           "owner": p.owner, "created_at": p.created_at, "house": p.house,
           "webhook": p.webhook_url.is_some()})
}

#[derive(Deserialize)]
struct RegisterReq {
    name: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    webhook_url: Option<String>,
    #[serde(default)]
    secret: Option<String>,
}

async fn register(State(app): App, headers: HeaderMap, Json(req): Json<RegisterReq>) -> Res<(StatusCode, Json<Value>)> {
    if !app.cfg.open_registration {
        let p = app.authenticate(&headers)?;
        if !p.is_admin() {
            return Err(ApiError::forbidden("registration is closed"));
        }
    }
    let name = req.name.trim().to_string();
    if name.is_empty() || name.len() > 64 {
        return Err(ApiError::bad_request("name must be 1..64 characters"));
    }
    let kind = req.kind.unwrap_or_else(|| "bot".into());
    if kind != "bot" && kind != "human" {
        return Err(ApiError::bad_request("kind must be bot or human"));
    }
    if name.starts_with("house:") {
        return Err(ApiError::bad_request("the house: prefix is reserved"));
    }
    if req.webhook_url.is_some() {
        if !app.cfg.enable_webhooks {
            return Err(ApiError::bad_request("webhook mode is disabled on this server (ARENA_ENABLE_WEBHOOKS)"));
        }
        if req.secret.as_deref().unwrap_or("").is_empty() {
            return Err(ApiError::bad_request("webhook_url requires a secret"));
        }
    }
    let token = format!("sk_{}", random_hex());
    let rec = PlayerRecord {
        player_id: new_id("p"),
        name,
        kind,
        owner: req.owner,
        created_at: now_iso(),
        house: false,
        token_hash: sha256_hex(token.as_bytes()),
        webhook_url: req.webhook_url,
        webhook_secret: req.secret,
    };
    app.store.put_player(&rec)?;
    let mut v = public_player(&rec);
    v["token"] = json!(token);
    v["api_key"] = json!(token);
    Ok((StatusCode::CREATED, Json(v)))
}

async fn list_players(State(app): App) -> Res<Json<Value>> {
    let v: Vec<Value> = app.store.list_players(500)?.iter().map(public_player).collect();
    Ok(Json(json!(v)))
}

async fn get_player(State(app): App, Path(id): Path<String>) -> Res<Json<Value>> {
    let id = if HouseRegistry::looks_like_house(&id) { house_id(&app.house.resolve(&id)?) } else { id };
    let p = app.store.get_player(&id)?.ok_or_else(|| ApiError::not_found("player"))?;
    let rs = app.store.results_of_player(&id)?;
    let mut v = public_player(&p);
    let hands: u64 = rs.iter().map(|r| r.hands).sum();
    let lb = leaderboard(&rs, &HashMap::from([(id.clone(), p.clone())]), &[], 0, true, None);
    v["stats"] = json!({"matches": rs.len(), "hands": hands,
        "mbb_per_game": lb.first().and_then(|r| r.score_mbb_per_hand),
        "ci95_mbb": lb.first().and_then(|r| r.ci95_mbb_per_hand)});
    Ok(Json(v))
}

async fn list_house(State(app): App) -> Json<Value> {
    Json(json!(app.house.bots.keys().map(|k| json!({"player_id": house_id(k), "name": k})).collect::<Vec<_>>()))
}

// ---- tables ----

async fn create_table(State(app): App, headers: HeaderMap, Json(cfg): Json<TableConfig>) -> Res<(StatusCode, Json<Value>)> {
    app.ensure_accepting()?;
    let who = app.authenticate(&headers)?;
    cfg.validate()?;
    if !who.is_admin() {
        let n = app.open_tables_of(&who.owner_id());
        if n >= app.cfg.max_tables_per_player {
            return Err(ApiError::rate_limited(
                format!("you already have {n} open tables (limit {}); close one with DELETE /v1/tables/{{id}}", app.cfg.max_tables_per_player),
                json!({"limit": "ARENA_MAX_TABLES_PER_PLAYER", "max": app.cfg.max_tables_per_player}),
            ));
        }
    }
    let id = new_id("t");
    let t = spawn_table(&app, id, who.owner_id(), cfg, None, 0);
    Ok((StatusCode::CREATED, Json(t.table_json(&app.hub))))
}

async fn list_tables(State(app): App) -> Json<Value> {
    let mut v: Vec<(String, Value)> = app
        .tables
        .read()
        .unwrap()
        .values()
        .map(|t| {
            let mut j = t.table_json(&app.hub);
            j["hand"] = Value::Null;
            (j["created_at"].as_str().unwrap_or("").to_string(), j)
        })
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    Json(json!(v.into_iter().map(|x| x.1).collect::<Vec<_>>()))
}

async fn get_table(State(app): App, Path(id): Path<String>) -> Res<Json<Value>> {
    Ok(Json(app.table(&id)?.table_json(&app.hub)))
}

#[derive(Deserialize)]
struct SitReq {
    #[serde(default)]
    player_id: Option<String>,
    #[serde(default)]
    house: Option<String>,
}

fn seat_player(app: &AppState, who: &Principal, owner: &str, req: &SitReq) -> Res<PlayerRecord> {
    let id = match (&req.house, &req.player_id) {
        (Some(h), _) => house_id(&app.house.resolve(h)?),
        (None, Some(p)) if HouseRegistry::looks_like_house(p) => house_id(&app.house.resolve(p)?),
        (None, Some(p)) => p.clone(),
        (None, None) => match who {
            Principal::Player(p) => p.player_id.clone(),
            Principal::Admin | Principal::Runner => return Err(ApiError::bad_request("player_id required")),
        },
    };
    let rec = app.store.get_player(&id)?.ok_or_else(|| ApiError::not_found("player"))?;
    let allowed = who.is_admin()
        || (rec.house && who.owner_id() == owner)
        || matches!(who, Principal::Player(p) if p.player_id == rec.player_id);
    if !allowed {
        return Err(ApiError::forbidden("only an admin or the player itself can seat a player (the table owner can seat house bots)"));
    }
    Ok(rec)
}

fn do_sit(app: &AppState, t: &crate::table::TableShared, seat: usize, rec: &PlayerRecord) -> Res<Value> {
    {
        let mut inner = t.inner.lock().unwrap();
        if inner.match_id.is_some() {
            return Err(ApiError::conflict("table_locked", "match tables are seated by the match"));
        }
        if seat >= inner.seats.len() {
            return Err(ApiError::bad_request(format!("seat must be 0..{}", inner.seats.len() - 1)));
        }
        if inner.seats[seat].player_id.is_some() {
            return Err(ApiError::conflict("seat_taken", "seat is taken"));
        }
        if !rec.house && inner.seats.iter().any(|s| s.player_id.as_deref() == Some(&rec.player_id)) {
            return Err(ApiError::conflict("seat_taken", "player already has a seat at this table"));
        }
        let stack = inner.config.starting_stack;
        inner.seats[seat] = SeatState {
            player_id: Some(rec.player_id.clone()),
            name: Some(rec.name.clone()),
            kind: rec.kind.clone(),
            house: rec.house,
            stack,
            stand_up: false,
        };
    }
    t.emit_seat_update(&app.hub, seat);
    t.wake.notify_one();
    let inner = t.inner.lock().unwrap();
    Ok(crate::table::TableShared::seat_json(&inner, seat, &app.hub))
}

async fn sit(State(app): App, headers: HeaderMap, Path((id, seat)): Path<(String, usize)>, Json(req): Json<SitReq>) -> Res<Json<Value>> {
    let who = app.authenticate(&headers)?;
    let t = app.table(&id)?;
    let owner = t.inner.lock().unwrap().owner.clone();
    app.ensure_accepting()?;
    let rec = seat_player(&app, &who, &owner, &req)?;
    if !rec.house && !who.is_admin() {
        let n = app.open_tables_of(&rec.player_id);
        if n >= app.cfg.max_tables_per_player {
            return Err(ApiError::rate_limited(
                format!("{} is already at or owns {n} open tables (limit {})", rec.player_id, app.cfg.max_tables_per_player),
                json!({"limit": "ARENA_MAX_TABLES_PER_PLAYER", "max": app.cfg.max_tables_per_player}),
            ));
        }
    }
    Ok(Json(do_sit(&app, &t, seat, &rec)?))
}

async fn stand(State(app): App, headers: HeaderMap, Path((id, seat)): Path<(String, usize)>) -> Res<Json<Value>> {
    let who = app.authenticate(&headers)?;
    let t = app.table(&id)?;
    {
        let mut inner = t.inner.lock().unwrap();
        if inner.match_id.is_some() {
            return Err(ApiError::conflict("table_locked", "match tables are seated by the match"));
        }
        let s = inner.seats.get(seat).ok_or_else(|| ApiError::bad_request("bad seat"))?.clone();
        let Some(pid) = s.player_id else { return Err(ApiError::conflict("not_seated", "seat is empty")) };
        let ok = who.is_admin() || inner.owner == who.owner_id() || pid == who.owner_id();
        if !ok {
            return Err(ApiError::forbidden("not allowed to stand this seat up"));
        }
        if inner.in_hand {
            inner.seats[seat].stand_up = true; // takes effect after the current hand
        } else {
            inner.seats[seat] = SeatState::empty();
        }
    }
    t.emit_seat_update(&app.hub, seat);
    let inner = t.inner.lock().unwrap();
    Ok(Json(crate::table::TableShared::seat_json(&inner, seat, &app.hub)))
}

#[derive(Deserialize)]
struct FillReq {
    bot: String,
}

/// Fill the empty seats with a house bot, or with the `ladder` line-up.
async fn fill(State(app): App, headers: HeaderMap, Path(id): Path<String>, Json(req): Json<FillReq>) -> Res<Json<Value>> {
    let who = app.authenticate(&headers)?;
    let t = app.table(&id)?;
    let owner = t.inner.lock().unwrap().owner.clone();
    if !who.is_admin() && who.owner_id() != owner {
        return Err(ApiError::forbidden("only the table owner can fill seats"));
    }
    let ids: Vec<String> = if req.bot == "ladder" { app.house.ladder()? } else { vec![house_id(&app.house.resolve(&req.bot)?)] };
    let empty: Vec<usize> = {
        let inner = t.inner.lock().unwrap();
        (0..inner.seats.len()).filter(|&i| inner.seats[i].player_id.is_none()).collect()
    };
    for (k, seat) in empty.into_iter().enumerate() {
        if req.bot == "ladder" && k >= ids.len() {
            break;
        }
        let pid = if req.bot == "ladder" { &ids[k] } else { &ids[0] };
        let rec = app.store.get_player(pid)?.ok_or_else(|| ApiError::not_found("house player"))?;
        do_sit(&app, &t, seat, &rec)?;
    }
    Ok(Json(t.table_json(&app.hub)))
}

// ---- matches ----

#[derive(Deserialize)]
struct MatchReq {
    #[serde(default)]
    table_config: Option<TableConfig>,
    #[serde(default)]
    participants: Vec<String>,
    /// `"ladder"` appends random, calling_station, 2x tight_aggressive, sharingan-1 to `participants`.
    #[serde(default)]
    field: Option<String>,
    hands_planned: u32,
    #[serde(default)]
    duplicate: Option<Value>,
    #[serde(default)]
    seed: Option<String>,
    /// "full" (default) | "cards" | "off"
    #[serde(default)]
    aivat: Option<String>,
    #[serde(default)]
    block_deals: Option<u32>,
    /// "hand_end" (default, per spec) | "match_end": withhold other players' cards until the match ends
    #[serde(default)]
    reveal: Option<String>,
    /// Play disjoint blocks of deals on this many tables in parallel (duplicate matches; default 1).
    #[serde(default)]
    lanes: Option<usize>,
}

/// Players may have a limited number of unfinished matches / benchmark runs at once (admin and runner: unlimited).
pub fn check_match_quota(app: &AppState, who: &Principal) -> Res<()> {
    if matches!(who, Principal::Player(_)) {
        let n = app.live_matches_of(&who.owner_id());
        if n >= app.cfg.max_matches_per_player {
            return Err(ApiError::rate_limited(
                format!("you already have {n} unfinished matches or runs (limit {})", app.cfg.max_matches_per_player),
                json!({"limit": "ARENA_MAX_MATCHES_PER_PLAYER", "max": app.cfg.max_matches_per_player}),
            ));
        }
    }
    Ok(())
}

fn match_json(rt: &MatchRuntime, app: &AppState) -> Value {
    let mut v = serde_json::to_value(rt.record()).unwrap();
    v["standings"] = serde_json::to_value(rt.standings()).unwrap();
    v["reveal"] = json!(if rt.hide_cards { "match_end" } else { "hand_end" });
    let _ = app;
    v
}

async fn create_match(State(app): App, headers: HeaderMap, Json(req): Json<MatchReq>) -> Res<(StatusCode, Json<Value>)> {
    app.ensure_accepting()?;
    let who = app.authenticate(&headers)?;
    check_match_quota(&app, &who)?;
    let mut parts = req.participants.clone();
    match req.field.as_deref() {
        None => {}
        Some("ladder") => parts.extend(app.house.ladder()?),
        Some(o) => return Err(ApiError::bad_request(format!("unknown field {o}"))),
    }
    if parts.len() != 6 {
        return Err(ApiError::bad_request(format!("need exactly 6 participants, got {}", parts.len())));
    }
    let mut ids = Vec::new();
    for p in &parts {
        let id = if HouseRegistry::looks_like_house(p) { house_id(&app.house.resolve(p)?) } else { p.clone() };
        let rec = app.store.get_player(&id)?.ok_or_else(|| ApiError::not_found(&format!("participant {p}")))?;
        if !rec.house && ids.contains(&rec.player_id) {
            return Err(ApiError::bad_request(format!("{} appears twice; only house bots can fill several seats", rec.player_id)));
        }
        ids.push(rec.player_id);
    }
    let mut cfg = req.table_config.unwrap_or_default();
    if cfg.stack_mode != StackMode::Reset {
        return Err(ApiError::bad_request("matches require stack_mode=reset"));
    }
    cfg.max_seats = 6;
    cfg.min_players_to_deal = 6;
    cfg.validate()?;
    let duplicate = match req.duplicate {
        None | Some(Value::Null) | Some(Value::Bool(false)) => None,
        Some(Value::Bool(true)) => Some(DuplicateConfig { enabled: true, rotations: 6, deck_from: "seed".into() }),
        Some(v) => {
            let d: DuplicateConfig = serde_json::from_value(v).map_err(|e| ApiError::bad_request(format!("duplicate: {e}")))?;
            if d.rotations != 6 || d.deck_from != "seed" {
                return Err(ApiError::bad_request("duplicate supports rotations=6, deck_from=seed"));
            }
            if d.enabled { Some(d) } else { None }
        }
    };
    if req.hands_planned == 0 || req.hands_planned > app.cfg.max_match_hands {
        return Err(ApiError::bad_request(format!("hands_planned must be 1..{}", app.cfg.max_match_hands))
            .with(json!({"limit": "ARENA_MAX_MATCH_HANDS", "max": app.cfg.max_match_hands})));
    }
    if duplicate.is_some() && req.hands_planned % 6 != 0 {
        return Err(ApiError::bad_request("duplicate matches need hands_planned to be a multiple of 6 (one deal = 6 hands)"));
    }
    let aivat = req.aivat.unwrap_or_else(|| "full".into());
    if !["full", "cards", "off"].contains(&aivat.as_str()) {
        return Err(ApiError::bad_request("aivat must be full, cards or off"));
    }
    let reveal = req.reveal.unwrap_or_else(|| "hand_end".into());
    if !["hand_end", "match_end"].contains(&reveal.as_str()) {
        return Err(ApiError::bad_request("reveal must be hand_end or match_end"));
    }
    let rt = matches::create(
        &app,
        NewMatch {
            owner: who.owner_id(),
            cfg,
            participants: ids,
            hands: req.hands_planned,
            duplicate,
            seed: req.seed.unwrap_or_else(|| random_hex()[..32].to_string()),
            aivat,
            block_deals: req.block_deals.unwrap_or(12),
            hide_cards: reveal == "match_end",
            lanes: req.lanes.unwrap_or(1),
            bench: None,
        },
    )?;
    Ok((StatusCode::CREATED, Json(match_json(&rt, &app))))
}

async fn list_matches(State(app): App) -> Res<Json<Value>> {
    let ids: Vec<MatchRecord> = app.store.list_matches(200)?;
    let v: Vec<Value> = ids
        .into_iter()
        .map(|m| {
            // live records win over stored ones
            let live = app.matches.read().unwrap().get(&m.match_id).map(|r| r.record());
            serde_json::to_value(live.unwrap_or(m)).unwrap()
        })
        .collect();
    Ok(Json(json!(v)))
}

async fn get_match(State(app): App, Path(id): Path<String>) -> Res<Json<Value>> {
    if let Ok(rt) = app.match_rt(&id) {
        return Ok(Json(match_json(&rt, &app)));
    }
    let m = app.store.get_match(&id)?.ok_or_else(|| ApiError::not_found("match"))?;
    let rows: Vec<StoredResult> = app.store.all_results()?.into_iter().filter(|r| r.match_id == id).collect();
    let mut v = serde_json::to_value(m)?;
    v["results"] = json!(rows);
    Ok(Json(v))
}

fn controls(app: &AppState, who: &Principal, rt: &MatchRuntime) -> Res<Arc<crate::table::TableShared>> {
    let t = app.table(&rt.table_id)?;
    let owner = t.inner.lock().unwrap().owner.clone();
    if !who.is_admin() && who.owner_id() != owner {
        return Err(ApiError::forbidden("only the match owner or an admin can control it"));
    }
    Ok(t)
}

async fn start_match(State(app): App, headers: HeaderMap, Path(id): Path<String>) -> Res<Json<Value>> {
    let who = app.authenticate(&headers)?;
    let rt = app.match_rt(&id)?;
    controls(&app, &who, &rt)?;
    matches::start(&app, &rt)?;
    Ok(Json(match_json(&rt, &app)))
}

async fn pause_match(State(app): App, headers: HeaderMap, Path(id): Path<String>) -> Res<Json<Value>> {
    let who = app.authenticate(&headers)?;
    let rt = app.match_rt(&id)?;
    controls(&app, &who, &rt)?;
    {
        let mut s = rt.state.lock().unwrap();
        if s.rec.status != "running" {
            return Err(ApiError::conflict("match_not_running", format!("match is {}", s.rec.status)));
        }
        s.rec.status = "paused".into();
        app.store.put_match(&s.rec)?;
    }
    Ok(Json(match_json(&rt, &app)))
}

async fn abort_match(State(app): App, headers: HeaderMap, Path(id): Path<String>) -> Res<Json<Value>> {
    let who = app.authenticate(&headers)?;
    let rt = app.match_rt(&id)?;
    controls(&app, &who, &rt)?;
    {
        let mut s = rt.state.lock().unwrap();
        if s.rec.status == "finished" || s.rec.status == "aborted" {
            return Err(ApiError::conflict("match_not_running", format!("match is {}", s.rec.status)));
        }
        s.abort = true;
        s.rec.status = "aborted".into();
        app.store.put_match(&s.rec)?;
    }
    matches::wake_all(&app, &rt);
    Ok(Json(match_json(&rt, &app)))
}

// ---- hands ----

/// Hand histories carry every hole card. While a match is live only its owner and admins may read them,
/// otherwise a participant could look up a deal's other rotations.
fn can_read_match_hands(app: &AppState, who: &Principal, match_id: &str) -> Res<()> {
    if who.is_admin() {
        return Ok(());
    }
    let m = app.match_rt(match_id).map(|r| r.record()).or_else(|_| {
        app.store.get_match(match_id)?.ok_or_else(|| ApiError::not_found("match"))
    })?;
    if m.status == "finished" || m.status == "aborted" {
        return Ok(());
    }
    let owner = app.tables.read().unwrap().get(&m.table_id).map(|t| t.inner.lock().unwrap().owner.clone());
    if owner.as_deref() == Some(&who.owner_id()) {
        return Ok(());
    }
    Err(ApiError::forbidden("hand histories of a running match are only visible to its owner"))
}

#[derive(Deserialize)]
struct HandsQ {
    #[serde(default)]
    after_hand_no: u32,
    #[serde(default)]
    limit: Option<u32>,
}

async fn match_hands(State(app): App, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HandsQ>) -> Res<Json<Value>> {
    let who = app.authenticate(&headers)?;
    can_read_match_hands(&app, &who, &id)?;
    app.writer.flush().await;
    let h = app.store.list_hands(&id, q.after_hand_no, q.limit.unwrap_or(500).clamp(1, 5000))?;
    Ok(Json(serde_json::to_value(h)?))
}

async fn match_hands_jsonl(State(app): App, headers: HeaderMap, Path(id): Path<String>) -> Res<Response> {
    let who = app.authenticate(&headers)?;
    can_read_match_hands(&app, &who, &id)?;
    app.writer.flush().await;
    let stream = futures_util::stream::unfold((app, id, 0u32, false), |(app, id, after, done)| async move {
        if done {
            return None;
        }
        let page = app.store.list_hands(&id, after, 500).unwrap_or_default();
        if page.is_empty() {
            return None;
        }
        let last = page.last().unwrap().hand_no;
        let mut buf = String::new();
        for h in &page {
            buf.push_str(&serde_json::to_string(h).unwrap_or_default());
            buf.push('\n');
        }
        let short = page.len() < 500;
        Some((Ok::<Bytes, Infallible>(Bytes::from(buf)), (app, id, last, short)))
    });
    Ok(([("content-type", "application/x-ndjson")], Body::from_stream(stream)).into_response())
}

async fn get_hand(State(app): App, headers: HeaderMap, Path(id): Path<String>) -> Res<Json<Value>> {
    let who = app.authenticate(&headers)?;
    app.writer.flush().await;
    let h = app.store.get_hand(&id)?.ok_or_else(|| ApiError::not_found("hand"))?;
    if let Some(m) = &h.match_id {
        can_read_match_hands(&app, &who, m)?;
    }
    Ok(Json(serde_json::to_value(h)?))
}

async fn get_hand_events(State(app): App, headers: HeaderMap, Path(id): Path<String>) -> Res<Json<Value>> {
    let who = app.authenticate(&headers)?;
    app.writer.flush().await;
    let h = app.store.get_hand(&id)?.ok_or_else(|| ApiError::not_found("hand"))?;
    if let Some(m) = &h.match_id {
        can_read_match_hands(&app, &who, m)?;
    }
    let ev = app.store.get_hand_events(&id)?.unwrap_or_default();
    Ok(Json(json!(ev)))
}

#[derive(Deserialize)]
struct LbQ {
    #[serde(default)]
    min_hands: Option<u64>,
    #[serde(default)]
    include_house: Option<bool>,
    /// Only bots with a run on this track, ranked by it (e.g. `ladder-v1`).
    #[serde(default)]
    track: Option<String>,
}

async fn get_leaderboard(State(app): App, Query(q): Query<LbQ>) -> Res<Json<Value>> {
    let results = app.store.all_results()?;
    let runs = app.store.list_runs(None, None, 100_000)?;
    let players: HashMap<String, PlayerRecord> = app.store.list_players(100_000)?.into_iter().map(|p| (p.player_id.clone(), p)).collect();
    let min = q.min_hands.unwrap_or(app.cfg.min_leaderboard_hands);
    let rows = leaderboard(&results, &players, &runs, min, q.include_house.unwrap_or(true), q.track.as_deref());
    Ok(Json(json!({"min_hands": min, "track": q.track, "rows": rows})))
}
