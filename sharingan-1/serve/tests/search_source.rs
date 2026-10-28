//! `source` honesty (off-policy histories, empty search averages) and rate-limit defaults.
//! Tiny blueprint, same as `tests/api.rs`; skipped when it is missing.

mod common;

use common::*;
use serde_json::{json, Value};
use sharingan_serve::config::Config;
use sharingan_serve::decide::Core;
use std::sync::{Arc, OnceLock};

macro_rules! need {
    ($e:expr) => {
        match $e {
            Some(x) => x,
            None => {
                eprintln!("SKIPPED: tiny blueprint not found (set SHARINGAN_TEST_BLUEPRINT)");
                return;
            }
        }
    };
}

/// Library-default rate limits (200 rps / burst 400) plus a tight per-key override for bob.
fn core_default_rate() -> Option<Arc<Core>> {
    static C: OnceLock<Option<Arc<Core>>> = OnceLock::new();
    C.get_or_init(|| {
        let mut c: Config = base_cfg()?;
        let d = Config::default();
        c.rate_rps = d.rate_rps;
        c.rate_burst = d.rate_burst;
        c.rate_overrides.insert("bob".into(), (1.0, 2.0));
        sharingan_serve::log::set_enabled(false);
        Some(Arc::new(Core::new(&c).expect("load tiny blueprint")))
    })
    .clone()
}

async fn act(app: &axum::Router, req: &Value) -> Value {
    let (s, _, v) = call(app, "POST", "/v1/act", Some(KEY), &req.to_string()).await;
    assert_eq!(s, 200, "{v}");
    v
}

/// Random hero actions are overwhelmingly off-policy: those requests must skip the search, say so,
/// and answer from the blueprint at once. Whatever is labelled `search` must carry no reason.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn off_policy_history_skips_search() {
    let core = need!(core_main());
    let app = app(&core);
    let spots = gen_spots(21, 40, STD, &|s| s.expect.street >= 2 && s.expect.n >= 2);
    let (mut off, mut searched) = (0, 0);
    for sp in &spots {
        let v = act(&app, &sp.req).await;
        check_response(sp, &v);
        match v["fallback_reason"].as_str() {
            Some("off_policy_history") => {
                off += 1;
                assert_eq!(v["source"], "blueprint", "{v}");
                assert!(v["timing"]["search_ms"].is_null(), "search must not run: {v}");
                assert!(v["timing"]["total_ms"].as_f64().unwrap() < 500.0, "blueprint answer must be fast: {v}");
            }
            Some("no_search_mass") => assert_eq!(v["source"], "blueprint", "{v}"),
            Some(_) => {}
            None => {
                if v["source"] == "search" {
                    searched += 1;
                }
            }
        }
    }
    eprintln!("off_policy_history: {off}, searched: {searched} of {}", spots.len());
    assert!(off >= 3, "expected several off-policy requests among random hero actions, got {off}");
    let (_, _, info) = call(&app, "GET", "/v1/info", Some(KEY), "").await;
    assert!(info["stats"]["off_policy_history"].as_u64().unwrap() >= off);
}

/// Turn spots whose hero history is on-policy really search: `source: "search"`, no fallback
/// reason, a search time. Every turn answer that is not a search says why.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn on_policy_turn_spot_searches() {
    let core = need!(core_main());
    let app = app(&core);
    let mut searched = 0;
    for sp in gen_spots(23, 60, STD, &|s| s.expect.street == 2 && s.expect.n >= 2) {
        let v = act(&app, &sp.req).await;
        check_response(&sp, &v);
        assert_eq!(v["street"], "turn", "{v}");
        if v["source"] == "search" {
            searched += 1;
            assert!(v["fallback_reason"].is_null(), "{v}");
            assert!(v["timing"]["search_ms"].as_f64().unwrap() > 0.0, "{v}");
        }
    }
    assert!(searched >= 3, "only {searched} turn spots searched");
}

/// Defaults are 200 rps / burst 400 per key (bulk callers), a per-key override applies only to
/// that key, and `/v1/info` reports the limits.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn default_rate_limit_and_per_key_override() {
    let core = need!(core_default_rate());
    let app = app(&core);
    let sp = &gen_spots(22, 1, STD, &|s| s.expect.street == 0)[0];
    let mut r = sp.req.clone();
    r["options"] = json!({"search": false, "seed": 1});
    let body = r.to_string();
    for i in 0..300 {
        let (s, _, v) = call(&app, "POST", "/v1/act", Some(KEY), &body).await;
        assert_eq!(s, 200, "request {i}: {v}");
    }
    // bob is capped at burst 2 by the override
    for i in 0..2 {
        let (s, _, v) = call(&app, "POST", "/v1/act", Some(KEY2), &body).await;
        assert_eq!(s, 200, "bob {i}: {v}");
    }
    let (s, _, v) = call(&app, "POST", "/v1/act", Some(KEY2), &body).await;
    assert_eq!((s, v["error"]["code"].as_str()), (429, Some("rate_limited")), "{v}");
    let (_, _, info) = call(&app, "GET", "/v1/info", Some(KEY), "").await;
    assert_eq!(info["limits"]["rate_rps"], 200.0);
    assert_eq!(info["limits"]["rate_burst"], 400.0);
}
