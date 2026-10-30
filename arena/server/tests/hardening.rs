//! Release hardening: match ownership and connection checks, the JSON error envelope on every rejection,
//! player-name validation, hand-page limits. In-process server on an ephemeral port, in-memory SQLite.

use arena_server::config::ServerConfig;
use arena_server::{start, RunningServer};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};

struct Srv {
    base: String,
    ws: String,
    rs: RunningServer,
    http: reqwest::Client,
}

async fn srv() -> Srv {
    let cfg = ServerConfig {
        rate_limit_rps: 0.0,
        sizing: "tiny".into(),
        bind: "127.0.0.1:0".into(),
        db_path: ":memory:".into(),
        admin_token: Some("admin".into()),
        hand_delay_ms: 0,
        reconnect_grace_ms: 200,
        ..Default::default()
    };
    let rs = start(cfg).await.unwrap();
    Srv { base: format!("http://{}", rs.addr), ws: format!("ws://{}", rs.addr), rs, http: reqwest::Client::new() }
}

fn err_code(v: &Value) -> &str {
    v["error"]["code"].as_str().unwrap_or("")
}

fn house() -> Vec<&'static str> {
    vec!["house:random", "house:calling_station", "house:tight_aggressive", "house:nit", "house:lag", "house:maniac"]
}

fn with_first(first: &str) -> Value {
    let mut p = house();
    p[0] = first;
    json!(p)
}

impl Srv {
    async fn req(&self, method: &str, path: &str, token: Option<&str>, body: Option<Value>) -> (u16, Value) {
        let m = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
        let mut r = self.http.request(m, format!("{}{}", self.base, path));
        if let Some(t) = token {
            r = r.bearer_auth(t);
        }
        if let Some(b) = body {
            r = r.json(&b);
        }
        let resp = r.send().await.unwrap();
        let st = resp.status().as_u16();
        let txt = resp.text().await.unwrap();
        (st, serde_json::from_str(&txt).unwrap_or(Value::String(txt)))
    }
    async fn post(&self, p: &str, t: Option<&str>, b: Value) -> (u16, Value) {
        self.req("POST", p, t, Some(b)).await
    }
    async fn get(&self, p: &str, t: Option<&str>) -> (u16, Value) {
        self.req("GET", p, t, None).await
    }
    async fn register(&self, name: &str) -> (String, String) {
        let (st, v) = self.post("/v1/bots", None, json!({"name": name, "kind": "bot"})).await;
        assert_eq!(st, 201, "{v}");
        (v["player_id"].as_str().unwrap().into(), v["token"].as_str().unwrap().into())
    }
    async fn wait_match(&self, id: &str) -> Value {
        for _ in 0..600 {
            let (_, v) = self.get(&format!("/v1/matches/{id}"), None).await;
            if v["status"] == "finished" || v["status"] == "aborted" {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("match did not finish");
    }
    /// Open /v1/play for the token and wait for the welcome; the returned sink keeps the socket (and the bot "connected") alive.
    async fn connect(&self, token: &str) -> tokio::task::JoinHandle<()> {
        let (ws, _) = connect_async(format!("{}/v1/play?token={token}", self.ws)).await.unwrap();
        let (mut w, mut r) = ws.split();
        w.send(Message::Text(json!({"v": 1, "type": "hello", "t_ms": 0, "data": {"client": "test"}}).to_string().into())).await.unwrap();
        while let Some(Ok(Message::Text(t))) = r.next().await {
            if serde_json::from_str::<Value>(t.as_str()).unwrap()["type"] == "welcome" {
                break;
            }
        }
        tokio::spawn(async move {
            let _keep = w;
            while r.next().await.is_some() {}
        })
    }
}

// ---------------------------------------------------------------------------------------------- 1: match ownership

#[tokio::test]
async fn nobody_can_run_a_match_that_scores_someone_elses_bot() {
    let s = srv().await;
    let (victim, _victim_tok) = s.register("victim").await;
    let (mine, mytok) = s.register("attacker").await;
    let body = |first: &str| json!({"participants": with_first(first), "hands_planned": 6, "table_config": {"decision_time_ms": 1000}});

    let (st, e) = s.post("/v1/matches", Some(&mytok), body(&victim)).await;
    assert_eq!((st, err_code(&e)), (403, "forbidden"), "{e}");
    assert!(e["error"]["message"].as_str().unwrap().contains("you can only run your own bot"), "{e}");
    // same wording as the benchmark endpoint (ownership is checked before the panel is resolved)
    let (st, e2) = s.post("/v1/benchmarks/ladder-v1/runs", Some(&mytok), json!({"player_id": victim, "hands": 6})).await;
    assert_eq!((st, err_code(&e2)), (403, "forbidden"), "{e2}");
    assert_eq!(e["error"]["message"], e2["error"]["message"]);
    // nothing was created, and the victim has no results
    let (_, list) = s.get("/v1/matches", None).await;
    assert_eq!(list.as_array().unwrap().len(), 0);
    let (_, v) = s.get(&format!("/v1/players/{victim}"), None).await;
    assert_eq!(v["stats"]["matches"], 0);

    // your own bot, with house bots around it, is fine; so is the admin entering anybody
    let (st, m) = s.post("/v1/matches", Some(&mytok), body(&mine)).await;
    assert_eq!(st, 201, "{m}");
    let (st, m2) = s.post("/v1/matches", Some("admin"), body(&victim)).await;
    assert_eq!(st, 201, "{m2}");
    // two different players in one match is admin-only as well (a second bot of yours cannot be told apart from a stranger's)
    let (other, _) = s.register("other").await;
    let mut two = house();
    two[0] = &mine;
    two[1] = &other;
    let (st, e) = s.post("/v1/matches", Some(&mytok), json!({"participants": two, "hands_planned": 6})).await;
    assert_eq!((st, err_code(&e)), (403, "forbidden"), "{e}");
    // a stranger cannot start the admin's match with the victim in it
    let mid = m2["match_id"].as_str().unwrap();
    assert_eq!(s.post(&format!("/v1/matches/{mid}/start"), Some(&mytok), json!({})).await.0, 403);
    // `owner` is a free-text label chosen at registration, so it grants nothing
    let (st, liar) = s.post("/v1/bots", None, json!({"name": "liar", "owner": "attacker"})).await;
    assert_eq!(st, 201);
    let (st, _) = s.post("/v1/matches", Some(&mytok), body(liar["player_id"].as_str().unwrap())).await;
    assert_eq!(st, 403);
}

// ---------------------------------------------------------------------------------------------- 2: start needs connected bots

#[tokio::test]
async fn start_refuses_a_disconnected_participant_unless_admin() {
    let s = srv().await;
    let (pid, tok) = s.register("offline").await;
    let (st, m) = s
        .post("/v1/matches", Some(&tok), json!({"participants": with_first(&pid), "hands_planned": 6, "table_config": {"decision_time_ms": 1000}}))
        .await;
    assert_eq!(st, 201, "{m}");
    let mid = m["match_id"].as_str().unwrap().to_string();
    let (st, e) = s.post(&format!("/v1/matches/{mid}/start"), Some(&tok), json!({})).await;
    assert_eq!((st, err_code(&e)), (409, "not_connected"), "{e}");
    assert_eq!(e["error"]["details"]["player_id"], json!(pid));
    let (_, now) = s.get(&format!("/v1/matches/{mid}"), None).await;
    assert_eq!((now["status"].as_str(), now["hands_played"].as_u64()), (Some("scheduled"), Some(0)), "nothing was played");
    // resume goes through the same check
    assert_eq!(s.post(&format!("/v1/matches/{mid}/resume"), Some(&tok), json!({})).await.0, 409);
    // connect, then it starts
    let _sock = s.connect(&tok).await;
    let (st, started) = s.post(&format!("/v1/matches/{mid}/start"), Some(&tok), json!({})).await;
    assert_eq!(st, 200, "{started}");
    assert_eq!(started["status"], "running");
    // starting a running match again stays a harmless no-op
    assert_eq!(s.post(&format!("/v1/matches/{mid}/start"), Some(&tok), json!({})).await.0, 200);
    assert_eq!(s.post(&format!("/v1/matches/{mid}/abort"), Some("admin"), json!({})).await.0, 200);
    s.wait_match(&mid).await;

    // the admin may start it without the bot being there (it plays on with auto-folds)
    let (st, m) = s.post("/v1/matches", Some("admin"), json!({"participants": with_first(&pid), "hands_planned": 6, "table_config": {"decision_time_ms": 1000}})).await;
    assert_eq!(st, 201, "{m}");
    let id2 = m["match_id"].as_str().unwrap();
    assert_eq!(s.post(&format!("/v1/matches/{id2}/start"), Some("admin"), json!({})).await.0, 200);
    s.post(&format!("/v1/matches/{id2}/abort"), Some("admin"), json!({})).await;
    s.wait_match(id2).await;
    // house-only matches need nobody connected
    let (st, m) = s.post("/v1/matches", Some(&tok), json!({"participants": house(), "hands_planned": 6, "table_config": {"decision_time_ms": 1000}})).await;
    assert_eq!(st, 201, "{m}");
    let id3 = m["match_id"].as_str().unwrap();
    assert_eq!(s.post(&format!("/v1/matches/{id3}/start"), Some(&tok), json!({})).await.0, 200);
    assert_eq!(s.wait_match(id3).await["status"], "finished");
    let _ = &s.rs;
}

// ---------------------------------------------------------------------------------------------- 5: JSON error envelope

async fn raw(s: &Srv, method: &str, path: &str, ctype: Option<&str>, body: &str, token: Option<&str>) -> (u16, String, Value, reqwest::header::HeaderMap) {
    let mut r = s.http.request(reqwest::Method::from_bytes(method.as_bytes()).unwrap(), format!("{}{}", s.base, path));
    if let Some(c) = ctype {
        r = r.header("content-type", c);
    }
    if let Some(t) = token {
        r = r.bearer_auth(t);
    }
    if !body.is_empty() {
        r = r.body(body.to_string());
    }
    let resp = r.send().await.unwrap();
    let st = resp.status().as_u16();
    let headers = resp.headers().clone();
    let ct = headers.get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let txt = resp.text().await.unwrap();
    let v: Value = serde_json::from_str(&txt).unwrap_or_else(|_| panic!("{method} {path}: body is not JSON ({st}): {txt:?}"));
    (st, ct, v, headers)
}

#[tokio::test]
async fn every_rejection_uses_the_json_error_envelope() {
    let s = srv().await;
    let (_, tok) = s.register("env").await;
    let j = Some("application/json");
    // (method, path, content-type, body, token, expected status, expected code)
    let cases: Vec<(&str, &str, Option<&str>, &str, Option<&str>, u16, &str)> = vec![
        ("POST", "/v1/bots", j, "{not json", None, 400, "bad_request"),
        ("POST", "/v1/bots", j, "{}", None, 400, "bad_request"),              // missing field `name`
        ("POST", "/v1/bots", j, r#"{"name": 5}"#, None, 400, "bad_request"),  // wrong type
        ("POST", "/v1/bots", Some("text/plain"), r#"{"name":"x"}"#, None, 415, "unsupported_media_type"),
        ("POST", "/v1/bots", None, r#"{"name":"x"}"#, None, 415, "unsupported_media_type"),
        ("POST", "/v1/tables", j, r#"{"max_seats": "six"}"#, Some(&tok), 400, "bad_request"),
        ("POST", "/v1/matches", j, r#"{"participants": []}"#, Some(&tok), 400, "bad_request"), // hands_planned missing
        ("GET", "/v1/matches/m_x/hands?limit=abc", None, "", Some(&tok), 400, "bad_request"),
        ("GET", "/v1/matches/m_x/hands?after_hand_no=-1", None, "", Some(&tok), 400, "bad_request"),
        ("GET", "/v1/leaderboard?min_hands=lots", None, "", None, 400, "bad_request"),
        ("POST", "/v1/tables/t_x/seats/notanumber", j, "{}", Some(&tok), 400, "bad_request"),
        ("GET", "/v1/nope", None, "", None, 404, "not_found"),
        ("GET", "/", None, "", None, 404, "not_found"),
        ("POST", "/v1/matches/m_x/start/extra", j, "{}", Some(&tok), 404, "not_found"),
        ("PUT", "/v1/bots", j, "{}", Some(&tok), 405, "method_not_allowed"),
        ("DELETE", "/v1/leaderboard", None, "", None, 405, "method_not_allowed"),
        ("GET", "/v1/matches/m_x/start", None, "", None, 405, "method_not_allowed"),
        // the existing envelope is untouched
        ("GET", "/v1/me", None, "", None, 401, "unauthorized"),
        ("GET", "/v1/matches/m_x", None, "", None, 404, "not_found"),
    ];
    for (method, path, ct, body, token, want_st, want_code) in cases {
        let (st, ctype, v, headers) = raw(&s, method, path, ct, body, token).await;
        assert_eq!((st, err_code(&v)), (want_st, want_code), "{method} {path}: {v}");
        assert!(ctype.starts_with("application/json"), "{method} {path}: content-type {ctype}");
        assert!(v["error"]["message"].as_str().map(|m| !m.is_empty()).unwrap_or(false), "{method} {path}: {v}");
        assert!(v["error"]["details"].is_object(), "{method} {path}: {v}");
        assert_eq!(headers.get("access-control-allow-origin").and_then(|h| h.to_str().ok()), Some("*"), "CORS survives the mapping");
        if want_st == 405 {
            assert!(headers.contains_key("allow"), "{method} {path}: Allow header kept");
        }
    }
    // oversized bodies keep their own envelope
    let big = "x".repeat(2 << 20);
    let (st, _, v, _) = raw(&s, "POST", "/v1/bots", j, &format!(r#"{{"name":"{big}"}}"#), None).await;
    assert_eq!((st, err_code(&v)), (413, "payload_too_large"));
}

// ---------------------------------------------------------------------------------------------- 6: names

#[tokio::test]
async fn player_names_reject_control_invisible_and_bidi_characters() {
    let s = srv().await;
    for bad in [
        "nul\u{0}byte", "line\nbreak", "cr\rx", "tab\tx", "esc\u{1b}[31m", "del\u{7f}", "c1\u{85}x", "\u{202e}evil", "a\u{202a}b", "a\u{202d}b", "a\u{2066}b", "a\u{2069}b",
        "zero\u{200b}width", "zwnj\u{200c}x", "zwj\u{200d}x", "lrm\u{200e}x", "rlm\u{200f}x", "bom\u{feff}x", "sep\u{2028}x", "wj\u{2060}x",
    ] {
        let (st, e) = s.post("/v1/bots", None, json!({"name": bad})).await;
        assert_eq!((st, err_code(&e)), (400, "bad_request"), "{bad:?}: {e}");
        let (st, _) = s.post("/v1/players", None, json!({"name": bad, "kind": "human"})).await;
        assert_eq!(st, 400, "{bad:?}");
    }
    // other stored free text
    assert_eq!(s.post("/v1/bots", None, json!({"name": "ok", "owner": "o\u{0}wner"})).await.0, 400);
    assert_eq!(s.post("/v1/bots", None, json!({"name": "ok", "owner": "x".repeat(129)})).await.0, 400);
    assert_eq!(s.post("/v1/bots", None, json!({"name": "ok", "owner": "alice@example.com"})).await.0, 201);
    // ordinary Unicode is fine, and the length is counted in characters
    for good in ["José Müller", "李小龍", "Zoë 🂡♠", "Ünïcödé-bot_1.0", "🤖", &"é".repeat(64), &"🤖".repeat(64)] {
        let (st, v) = s.post("/v1/bots", None, json!({"name": good})).await;
        assert_eq!(st, 201, "{good:?}: {v}");
        assert_eq!(v["name"], json!(good));
    }
    for bad in ["", "   ", &"é".repeat(65), &"a".repeat(65)] {
        assert_eq!(s.post("/v1/bots", None, json!({"name": bad})).await.0, 400, "{bad:?}");
    }
    // seeds are stored text too
    let (_, tok) = s.register("seeder").await;
    let (st, _) = s.post("/v1/matches", Some("admin"), json!({"participants": house(), "hands_planned": 6, "seed": "a\u{0}b"})).await;
    assert_eq!(st, 400);
    let _ = tok;
}

// ---------------------------------------------------------------------------------------------- 8: paging and closed tables

#[tokio::test]
async fn hands_page_is_capped_at_500_and_zero_is_rejected() {
    let s = srv().await;
    let (st, m) = s
        .post("/v1/matches", Some("admin"), json!({"participants": house(), "hands_planned": 606, "aivat": "off", "table_config": {"decision_time_ms": 1000}}))
        .await;
    assert_eq!(st, 201, "{m}");
    let id = m["match_id"].as_str().unwrap().to_string();
    assert_eq!(s.post(&format!("/v1/matches/{id}/start"), Some("admin"), json!({})).await.0, 200);
    assert_eq!(s.wait_match(&id).await["status"], "finished");
    let n = |v: &Value| v.as_array().unwrap().len();
    let (st, h) = s.get(&format!("/v1/matches/{id}/hands"), Some("admin")).await;
    assert_eq!((st, n(&h)), (200, 500), "default page is 500");
    let (_, h) = s.get(&format!("/v1/matches/{id}/hands?limit=100000"), Some("admin")).await;
    assert_eq!(n(&h), 500, "limit is capped at 500");
    let (_, h) = s.get(&format!("/v1/matches/{id}/hands?limit=2"), Some("admin")).await;
    assert_eq!(n(&h), 2);
    let (_, h) = s.get(&format!("/v1/matches/{id}/hands?after_hand_no=500&limit=500"), Some("admin")).await;
    assert_eq!(n(&h), 106, "paging continues after the cap");
    let (st, e) = s.get(&format!("/v1/matches/{id}/hands?limit=0"), Some("admin")).await;
    assert_eq!((st, err_code(&e)), (400, "bad_request"), "{e}");
}

#[tokio::test]
async fn closing_a_closed_table_is_idempotent() {
    let s = srv().await;
    let (st, t) = s.post("/v1/tables", Some("admin"), json!({"decision_time_ms": 1000})).await;
    assert_eq!(st, 201, "{t}");
    let id = t["table_id"].as_str().unwrap();
    for _ in 0..2 {
        let (st, c) = s.req("DELETE", &format!("/v1/tables/{id}"), Some("admin"), None).await;
        assert_eq!(st, 200, "{c}");
        assert_eq!(c["status"], "closed");
    }
}
