//! HTTP layer: routes, Bearer auth, per-key token-bucket rate limit, JSON logs.
//!
//! Bearer = a configured API key, or (with `--arena-auth-url`) any arena player token: the arena's `GET /v1/me`
//! vouches for it once (cached), and that player gets the public rate limit.
//!
//! | route | auth | |
//! |---|---|---|
//! | `GET /v1/health` | no | liveness |
//! | `GET /v1/info` | yes | model id, sizing, search config, stats |
//! | `POST /v1/act` | yes | one decision |
//! | `POST /v1/act/batch` | yes | up to `max_batch` decisions; per-item errors, HTTP 200 |

use crate::api::{ActRequest, ActResponse, BatchRequest};
use crate::decide::Core;
use crate::error::ApiError;
use crate::log;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::Semaphore;

struct Bucket {
    tokens: f64,
    last: Instant,
}

pub struct AppState {
    pub core: Arc<Core>,
    /// key -> principal name
    keys: HashMap<String, String>,
    limiter: Mutex<HashMap<String, Bucket>>,
    inflight: Arc<Semaphore>,
    /// arena token -> (player id, or None if the arena rejected it; checked at)
    players: Mutex<HashMap<String, (Option<String>, Instant)>>,
}

const PLAYER_TTL_S: u64 = 300;
const REJECT_TTL_S: u64 = 30;

impl AppState {
    pub fn new(core: Arc<Core>) -> Arc<AppState> {
        let keys = core.cfg.api_keys.iter().map(|(n, k)| (k.clone(), n.clone())).collect();
        let inflight = Arc::new(Semaphore::new(core.cfg.max_inflight));
        Arc::new(AppState { core, keys, limiter: Mutex::new(HashMap::new()), inflight, players: Mutex::new(HashMap::new()) })
    }

    /// Take `cost` tokens from `who`'s bucket. `Err(retry_after_secs)` when empty.
    fn take(&self, who: &str, cost: f64) -> Result<(), f64> {
        let cfg = &self.core.cfg;
        let default = if who.starts_with("player:") { (cfg.public_rps, cfg.public_burst) } else { (cfg.rate_rps, cfg.rate_burst) };
        let (rps, burst) = cfg.rate_overrides.get(who).copied().unwrap_or(default);
        let cost = cost.min(burst);
        let mut m = self.limiter.lock().unwrap();
        let now = Instant::now();
        let b = m.entry(who.to_string()).or_insert(Bucket { tokens: burst, last: now });
        b.tokens = (b.tokens + now.duration_since(b.last).as_secs_f64() * rps).min(burst);
        b.last = now;
        if b.tokens >= cost {
            b.tokens -= cost;
            Ok(())
        } else {
            Err((cost - b.tokens) / rps)
        }
    }

    /// The arena player behind `token` (cached). `Err` only when the arena could not be asked.
    async fn arena_player(&self, url: &str, token: &str) -> Result<Option<String>, ApiError> {
        if let Some((p, at)) = self.players.lock().unwrap().get(token) {
            let ttl = if p.is_some() { PLAYER_TTL_S } else { REJECT_TTL_S };
            if at.elapsed().as_secs() < ttl {
                return Ok(p.clone());
            }
        }
        let (u, t) = (format!("{url}/v1/me"), token.to_string());
        let res = tokio::task::spawn_blocking(move || {
            match ureq::get(&u).set("authorization", &format!("Bearer {t}")).timeout(std::time::Duration::from_secs(3)).call() {
                Ok(r) => Ok(r.into_json::<Value>().ok().and_then(|v| v["player_id"].as_str().map(String::from))),
                Err(ureq::Error::Status(401 | 403, _)) => Ok(None),
                Err(e) => Err(e.to_string()),
            }
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
        let p = res.map_err(|e| {
            log::emit("error", "arena_auth", json!({"error": e}));
            ApiError::new(503, "auth_unavailable", "could not verify the token right now; retry shortly")
        })?;
        let mut m = self.players.lock().unwrap();
        if m.len() > 100_000 {
            m.clear();
        }
        m.insert(token.to_string(), (p.clone(), Instant::now()));
        Ok(p)
    }

    #[allow(clippy::result_large_err)]
    async fn authenticate(&self, headers: &HeaderMap, cost: f64) -> Result<String, ApiError> {
        let who = if self.core.cfg.auth {
            let tok = headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")))
                .map(str::trim)
                .filter(|t| !t.is_empty());
            let unauthorized = || ApiError::new(401, "unauthorized", "missing or invalid key (send `Authorization: Bearer <key>`; an arena bot token works)");
            match (tok, tok.and_then(|t| self.keys.get(t))) {
                (_, Some(n)) => n.clone(),
                (Some(t), None) => match &self.core.cfg.arena_auth_url {
                    Some(url) => match self.arena_player(url, t).await? {
                        Some(pid) => format!("player:{pid}"),
                        None => return Err(unauthorized()),
                    },
                    None => return Err(unauthorized()),
                },
                (None, None) => return Err(unauthorized()),
            }
        } else {
            "anonymous".to_string()
        };
        self.take(&who, cost).map_err(|wait| {
            ApiError::new(429, "rate_limited", format!("rate limit exceeded; retry in {:.1}s", wait)).with(json!({"retry_after_s": (wait * 10.0).ceil() / 10.0}))
        })?;
        Ok(who)
    }
}

fn respond(status: u16, body: &Value, rid: Option<&str>, retry_after: Option<f64>) -> Response {
    let mut r = (StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), body.to_string()).into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(id) = rid.and_then(|i| HeaderValue::from_str(i).ok()) {
        h.insert(HeaderName::from_static("x-request-id"), id);
    }
    if status == 401 {
        h.insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    }
    if let Some(s) = retry_after {
        if let Ok(v) = HeaderValue::from_str(&format!("{}", s.ceil().max(1.0) as u64)) {
            h.insert(header::RETRY_AFTER, v);
        }
    }
    r
}

fn error_response(e: &ApiError, rid: Option<&str>) -> Response {
    let retry = e.details.as_ref().and_then(|d| d.get("retry_after_s")).and_then(Value::as_f64);
    respond(e.status, &e.body(), rid, retry)
}

pub fn router(st: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/info", get(info))
        .route("/v1/act", post(act))
        .route("/v1/act/batch", post(batch))
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(8 << 20))
        .with_state(st)
}

async fn not_found() -> Response {
    error_response(&ApiError::new(404, "not_found", "no such route; see GET /v1/info"), None)
}

async fn health(State(st): State<Arc<AppState>>) -> Response {
    respond(200, &st.core.health(), None, None)
}

async fn info(State(st): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    match st.authenticate(&headers, 1.0).await {
        Ok(_) => respond(200, &st.core.info(), None, None),
        Err(e) => error_response(&e, None),
    }
}

async fn run_one(st: &Arc<AppState>, req: ActRequest, rid: String) -> Result<ActResponse, ApiError> {
    let permit = st
        .inflight
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::new(503, "overloaded", format!("more than {} decisions in flight; retry shortly", st.core.cfg.max_inflight)))?;
    let core = st.core.clone();
    let r = tokio::task::spawn_blocking(move || {
        let _p = permit;
        std::panic::catch_unwind(AssertUnwindSafe(|| core.decide(&req, rid))).unwrap_or_else(|p| {
            let m = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default();
            Err(ApiError::internal(format!("decision panicked: {m}")))
        })
    })
    .await;
    r.unwrap_or_else(|e| Err(ApiError::internal(format!("worker failed: {e}"))))
}

fn log_act(rid: &str, who: &str, t0: Instant, res: &Result<ActResponse, ApiError>, extra: Value) {
    let ms = (t0.elapsed().as_secs_f64() * 1e3 * 10.0).round() / 10.0;
    let mut f = match res {
        Ok(r) => json!({
            "request_id": rid, "key": who, "status": 200, "latency_ms": ms, "street": r.street, "source": r.source,
            "action": r.action.kind, "to": r.action.to, "fallback_reason": r.fallback_reason, "search_ms": r.timing.search_ms,
            "hand_id": r.hand_id,
        }),
        Err(e) => json!({"request_id": rid, "key": who, "status": e.status, "latency_ms": ms, "code": e.code, "message": e.message}),
    };
    if let (Some(m), Value::Object(x)) = (f.as_object_mut(), extra) {
        m.extend(x);
    }
    let lvl = match res {
        Ok(r) if r.source == "fallback" => "warn",
        Ok(_) => "info",
        Err(e) if e.status >= 500 => "error",
        Err(_) => "warn",
    };
    log::emit(lvl, "act", f);
}

async fn act(State(st): State<Arc<AppState>>, headers: HeaderMap, body: Bytes) -> Response {
    let t0 = Instant::now();
    let rid = st.core.next_request_id();
    let who = match st.authenticate(&headers, 1.0).await {
        Ok(w) => w,
        Err(e) => {
            log::emit("warn", "auth", json!({"request_id": rid, "status": e.status, "code": e.code}));
            return error_response(&e, Some(&rid));
        }
    };
    let req: ActRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            let err = ApiError::bad_request(format!("invalid request body: {e}"));
            log_act(&rid, &who, t0, &Err(err.clone()), json!({}));
            return error_response(&err, Some(&rid));
        }
    };
    let res = run_one(&st, req, rid.clone()).await;
    log_act(&rid, &who, t0, &res, json!({}));
    match res {
        Ok(r) => respond(200, &serde_json::to_value(&r).unwrap_or(Value::Null), Some(&rid), None),
        Err(e) => error_response(&e, Some(&rid)),
    }
}

async fn batch(State(st): State<Arc<AppState>>, headers: HeaderMap, body: Bytes) -> Response {
    let t0 = Instant::now();
    let rid = st.core.next_request_id();
    let who = match st.authenticate(&headers, 1.0).await {
        Ok(w) => w,
        Err(e) => {
            log::emit("warn", "auth", json!({"request_id": rid, "status": e.status, "code": e.code}));
            return error_response(&e, Some(&rid));
        }
    };
    let b: BatchRequest = match serde_json::from_slice(&body) {
        Ok(b) => b,
        Err(e) => return error_response(&ApiError::bad_request(format!("invalid batch body (expected {{\"requests\": [...]}}): {e}")), Some(&rid)),
    };
    let n = b.requests.len();
    let max = st.core.cfg.max_batch;
    if n == 0 || n > max {
        let e = ApiError::new(if n == 0 { 400 } else { 413 }, "bad_batch", format!("batch must contain 1..={max} requests, got {n}")).with(json!({"max_batch": max}));
        return error_response(&e, Some(&rid));
    }
    if n > 1 {
        if let Err(wait) = st.take(&who, (n - 1) as f64) {
            let e = ApiError::new(429, "rate_limited", format!("rate limit exceeded; retry in {wait:.1}s")).with(json!({"retry_after_s": (wait * 10.0).ceil() / 10.0}));
            return error_response(&e, Some(&rid));
        }
    }
    // Items run concurrently, but no more than the search concurrency at once, so queued items do not eat
    // their search budget waiting for a slot.
    let par = Arc::new(Semaphore::new(st.core.cfg.search_concurrency.max(1)));
    let mut handles = Vec::with_capacity(n);
    for (i, req) in b.requests.into_iter().enumerate() {
        let (st2, par2, id) = (st.clone(), par.clone(), format!("{rid}.{i}"));
        handles.push(tokio::spawn(async move {
            let _p = par2.acquire_owned().await;
            let t = Instant::now();
            let res = match serde_json::from_value::<ActRequest>(req) {
                Ok(r) => run_one(&st2, r, id.clone()).await,
                Err(e) => Err(ApiError::bad_request(format!("requests[{i}]: invalid request: {e}"))),
            };
            (id, t, res)
        }));
    }
    let mut results = Vec::with_capacity(n);
    let mut errors = 0;
    for h in handles {
        match h.await {
            Ok((id, t, res)) => {
                log_act(&id, &who, t, &res, json!({"batch": rid}));
                match res {
                    Ok(r) => results.push(json!({"ok": true, "response": r})),
                    Err(e) => {
                        errors += 1;
                        results.push(json!({"ok": false, "error": e.body()["error"]}));
                    }
                }
            }
            Err(e) => {
                errors += 1;
                results.push(json!({"ok": false, "error": ApiError::internal(format!("task failed: {e}")).body()["error"]}));
            }
        }
    }
    log::info("batch", json!({"request_id": rid, "key": who, "n": n, "errors": errors, "latency_ms": t0.elapsed().as_millis() as u64}));
    respond(200, &json!({"request_id": rid, "count": n, "errors": errors, "results": results}), Some(&rid), None)
}

/// Bind and serve until `shutdown` resolves.
pub async fn serve(st: Arc<AppState>, listener: tokio::net::TcpListener, shutdown: impl std::future::Future<Output = ()> + Send + 'static) -> std::io::Result<()> {
    axum::serve(listener, router(st)).with_graceful_shutdown(shutdown).await
}
