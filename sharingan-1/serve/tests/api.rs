//! Integration tests over a TINY blueprint (see README: `pluribus train --sizing tiny` + `compact`).
//! Skipped (with a loud message) when the blueprint file is missing:
//! `SHARINGAN_TEST_BLUEPRINT` or `~/dev-serve/tinybp/tiny.plcp`.

mod common;

use cards::Rng;
use common::*;
use serde_json::{json, Value};
use sharingan_serve::api::ActRequest;
use sharingan_serve::replay::Hand;
use std::time::Instant;

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

fn no_search(mut req: Value) -> String {
    req["options"] = json!({"search": false, "seed": 7});
    req.to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn health_and_info() {
    let core = need!(core_main());
    let app = app(&core);
    let (s, _, v) = call(&app, "GET", "/v1/health", None, "").await;
    assert_eq!(s, 200);
    assert_eq!(v["status"], "ok");
    // info needs a key
    let (s, _, _) = call(&app, "GET", "/v1/info", None, "").await;
    assert_eq!(s, 401);
    let (s, _, v) = call(&app, "GET", "/v1/info", Some(KEY), "").await;
    assert_eq!(s, 200);
    assert_eq!(v["model_id"], "sharingan-1");
    assert_eq!(v["sizing"], "tiny");
    assert_eq!(v["search"]["flop"], false);
    assert_eq!(v["search"]["turn"], true);
    assert_eq!(v["search"]["river"], true);
    assert_eq!(v["search"]["iters"], 1500);
    assert_eq!(v["search"]["budget_ms"], 3000);
    assert_eq!(v["max_stack_bb"], 150.0);
    assert!(v["blueprint"]["infosets"].as_u64().unwrap() > 1_000_000);
    let (s, _, v) = call(&app, "GET", "/v1/nope", Some(KEY), "").await;
    assert_eq!((s, v["error"]["code"].as_str()), (404, Some("not_found")));
}

/// Legal answers for thousands of random histories (all player counts, button positions, stack depths,
/// blind levels incl. ones that do not divide 100), checked against an independent nlhe oracle.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn legal_actions_for_many_random_histories() {
    let core = need!(core_main());
    let spots = gen_spots(1, 4000, STD, &|_| true);
    assert!(spots.len() >= 4000);
    let mut by_street = [0usize; 4];
    let mut by_n = [0usize; 7];
    let mut desync = 0;
    let t = Instant::now();
    for (i, sp) in spots.iter().enumerate() {
        by_street[sp.expect.street] += 1;
        by_n[sp.expect.n] += 1;
        let req: ActRequest = serde_json::from_str(&no_search(sp.req.clone())).unwrap();
        let resp = core.decide(&req, format!("t{i}")).unwrap_or_else(|e| panic!("{e}: {}", sp.req));
        let v = serde_json::to_value(&resp).unwrap();
        if v["source"] == "fallback" {
            // only allowed for blinds that do not divide 100 (inexact chip scaling), and always legal
            assert!(100 % sp.expect.bb != 0, "unexpected fallback: {v} for {}", sp.req);
            desync += 1;
            if desync <= 3 {
                eprintln!("fallback example: {} {}", v["fallback_reason"], sp.req);
            }
        } else {
            assert_eq!(v["source"], "blueprint", "{}", sp.req);
        }
        check_response(sp, &v);
    }
    eprintln!("4000 blueprint decisions in {:.1}s; streets {by_street:?}; players {by_n:?}", t.elapsed().as_secs_f64());
    eprintln!("inexact-scale fallbacks: {desync} / 4000");
    assert!(by_street.iter().all(|&c| c > 100), "streets covered: {by_street:?}");
    assert!((2..=6).all(|n| by_n[n] > 100), "player counts covered: {by_n:?}");
}

/// Same through HTTP (auth, JSON, headers), plus determinism for a fixed seed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn act_over_http_is_legal_and_seed_deterministic() {
    let core = need!(core_main());
    let app = app(&core);
    for (i, sp) in gen_spots(2, 300, STD, &|_| true).iter().enumerate() {
        let body = no_search(sp.req.clone());
        let (s, h, v) = call(&app, "POST", "/v1/act", Some(KEY), &body).await;
        assert_eq!(s, 200, "{v}");
        assert!(h.get("x-request-id").is_some());
        check_response(sp, &v);
        if i % 10 == 0 {
            let (_, _, v2) = call(&app, "POST", "/v1/act", Some(KEY), &body).await;
            assert_eq!(v["action"], v2["action"], "same seed, same request -> same action");
            assert_eq!(v["probs"], v2["probs"]);
        }
    }
    // sample:false returns the most likely action
    let sp = &gen_spots(3, 1, STD, &|s| s.expect.street == 0)[0];
    let mut r = sp.req.clone();
    r["options"] = json!({"search": false, "sample": false});
    let (_, _, v) = call(&app, "POST", "/v1/act", Some(KEY), &r.to_string()).await;
    let top = &v["probs"][0];
    assert_eq!(v["action"]["type"], top["type"]);
    // include_probs:false drops the distribution
    r["options"] = json!({"search": false, "include_probs": false});
    let (_, _, v) = call(&app, "POST", "/v1/act", Some(KEY), &r.to_string()).await;
    assert!(v.get("probs").is_none());
}

/// Turn / river spots run the real-time search (flop does not: `flop=0`) and stay legal.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn search_on_turn_and_river_and_not_on_flop() {
    let core = need!(core_main());
    let app = app(&core);
    let mut searched = 0;
    let mut spots = gen_spots(4, 150, STD, &|s| s.expect.street >= 2 && s.expect.n >= 2);
    spots.extend(gen_spots(5, 10, STD, &|s| s.expect.street == 1));
    for sp in &spots {
        let t = Instant::now();
        let (s, _, v) = call(&app, "POST", "/v1/act", Some(KEY), &sp.req.to_string()).await;
        assert_eq!(s, 200, "{v}");
        check_response(sp, &v);
        assert!(t.elapsed().as_millis() < 3500 + 500, "took {:?}", t.elapsed());
        match (sp.expect.street, v["source"].as_str().unwrap()) {
            (1, src) => assert_eq!(src, "blueprint", "flop must not search: {v}"),
            (_, "search") => {
                searched += 1;
                assert!(v["timing"]["search_ms"].as_f64().unwrap() > 0.0);
            }
            (_, "blueprint") => {} // off_policy_history / no_search_mass: no search, or empty average
            (_, other) => panic!("unexpected source {other}: {v}"),
        }
    }
    assert!(searched >= 8, "only {searched} of {} turn/river decisions ran a search", spots.len() - 10);
    let (_, _, info) = call(&app, "GET", "/v1/info", Some(KEY), "").await;
    assert!(info["stats"]["searched"].as_u64().unwrap() >= searched);
}

/// A search that cannot finish inside the budget falls back to the blueprint action, in time; the
/// abandoned search keeps its slot, so the next search is skipped (busy) rather than piling up.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn time_budget_fallback_to_blueprint() {
    let core = need!(core_slow());
    let app = app(&core);
    let sp = &searchable_river_spots(6, 2).await[0];
    let t = Instant::now();
    let (s, _, v) = call(&app, "POST", "/v1/act", Some(KEY), &sp.req.to_string()).await;
    let el = t.elapsed().as_millis();
    assert_eq!(s, 200, "{v}");
    check_response(sp, &v);
    assert_eq!(v["source"], "fallback", "{v}");
    assert!(v["fallback_reason"].as_str().unwrap().starts_with("search_timeout>300ms"), "{v}");
    assert!(el >= 290 && el < 1200, "answered in {el} ms (budget 300, search sleeps 1500)");
    assert!(v["timing"]["search_ms"].as_f64().unwrap() >= 290.0);
    // the 1-slot gate is still held by the abandoned search -> next river request skips the search at once
    let sp2 = &searchable_river_spots(7, 2).await[1];
    let t = Instant::now();
    let (s, _, v) = call(&app, "POST", "/v1/act", Some(KEY), &sp2.req.to_string()).await;
    assert_eq!(s, 200);
    check_response(sp2, &v);
    assert_eq!(v["source"], "fallback");
    assert!(v["fallback_reason"].as_str().unwrap().starts_with("search_busy"), "{v}");
    assert!(t.elapsed().as_millis() < 250);
    // blueprint-only requests are unaffected
    let (s, _, v) = call(&app, "POST", "/v1/act", Some(KEY), &no_search(sp2.req.clone())).await;
    assert_eq!(s, 200);
    assert_eq!(v["source"], "blueprint");
    // per-request budget can only lower the server's
    let mut r = sp.req.clone();
    r["options"] = json!({"time_budget_ms": 100000, "seed": 1});
    let (_, _, v) = call(&app, "POST", "/v1/act", Some(KEY), &r.to_string()).await;
    assert_eq!(v["timing"]["budget_ms"], 300);
    // let the abandoned searches end so the other tests' threads are clean
    std::thread::sleep(std::time::Duration::from_millis(1700));
    assert_eq!(core.gate().in_use(), 0, "permit released when the abandoned search ended");
    let (_, _, info) = call(&app, "GET", "/v1/info", Some(KEY), "").await;
    assert!(info["stats"]["search_timeouts"].as_u64().unwrap() >= 1);
    assert!(info["stats"]["search_busy"].as_u64().unwrap() >= 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn batch_endpoint() {
    let core = need!(core_main());
    let app = app(&core);
    let good = gen_spots(8, 6, STD, &|_| true);
    let mut reqs: Vec<Value> = good.iter().map(|s| serde_json::from_str(&no_search(s.req.clone())).unwrap()).collect();
    // item 2: the hero raises one chip above the maximum (bad_amount, reported at that history index)
    let (bad_amount, bad_idx) = {
        let sp = gen_spots(9, 1, STD, &|s| s.expect.raise.is_some() && s.expect.street == 0).remove(0);
        let mut r = sp.req.clone();
        let hi = sp.expect.raise.unwrap().1;
        let idx = r["history"].as_array().unwrap().len();
        let hero = r["hero_seat"].clone();
        r["history"].as_array_mut().unwrap().push(json!({"seat": hero, "type": "raise", "to": hi + 1}));
        (r, idx)
    };
    reqs.insert(2, bad_amount);
    let n = reqs.len();
    let body = json!({"requests": reqs}).to_string();
    let (s, _, v) = call(&app, "POST", "/v1/act/batch", Some(KEY), &body).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["count"], n);
    let res = v["results"].as_array().unwrap();
    assert_eq!(res.len(), n);
    for (i, r) in res.iter().enumerate() {
        if i == 2 {
            assert_eq!(r["ok"], false, "{r}");
            assert_eq!(r["error"]["code"], "bad_amount");
            assert_eq!(r["error"]["details"]["index"].as_u64().unwrap() as usize, bad_idx, "{r}");
        } else {
            let sp = &good[if i < 2 { i } else { i - 1 }];
            assert_eq!(r["ok"], true, "{r}");
            check_response(sp, &r["response"]);
        }
    }
    assert_eq!(v["errors"], 1);
    // size limits and shape
    let (s, _, v) = call(&app, "POST", "/v1/act/batch", Some(KEY), r#"{"requests":[]}"#).await;
    assert_eq!((s, v["error"]["code"].as_str()), (400, Some("bad_batch")));
    let many: Vec<Value> = (0..65).map(|_| json!({})).collect();
    let (s, _, v) = call(&app, "POST", "/v1/act/batch", Some(KEY), &json!({"requests": many}).to_string()).await;
    assert_eq!((s, v["error"]["code"].as_str()), (413, Some("bad_batch")));
    let (s, _, v) = call(&app, "POST", "/v1/act/batch", Some(KEY), "not json").await;
    assert_eq!((s, v["error"]["code"].as_str()), (400, Some("bad_request")));
    // a malformed item fails only itself
    let (s, _, v) = call(&app, "POST", "/v1/act/batch", Some(KEY), r#"{"requests":[{"nope":1}]}"#).await;
    assert_eq!(s, 200);
    assert_eq!((v["results"][0]["ok"].as_bool(), v["results"][0]["error"]["code"].as_str()), (Some(false), Some("bad_request")));
    let (s, _, _) = call(&app, "POST", "/v1/act/batch", None, &body).await;
    assert_eq!(s, 401);
    // a batch with searching items completes and every item is legal
    let sp = gen_spots(10, 6, STD, &|s| s.expect.street >= 2);
    let reqs: Vec<Value> = sp.iter().map(|s| s.req.clone()).collect();
    let (s, _, v) = call(&app, "POST", "/v1/act/batch", Some(KEY), &json!({"requests": reqs}).to_string()).await;
    assert_eq!(s, 200);
    for (r, spot) in v["results"].as_array().unwrap().iter().zip(&sp) {
        assert_eq!(r["ok"], true, "{r}");
        check_response(spot, &r["response"]);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn auth_and_rate_limit() {
    let core = need!(core_limited());
    let app = app(&core);
    let sp = &gen_spots(11, 1, STD, &|_| true)[0];
    let body = no_search(sp.req.clone());
    let (s, h, v) = call(&app, "POST", "/v1/act", None, &body).await;
    assert_eq!((s, v["error"]["code"].as_str()), (401, Some("unauthorized")));
    assert_eq!(h.get("www-authenticate").unwrap(), "Bearer");
    let (s, _, _) = call(&app, "POST", "/v1/act", Some("wrong-key"), &body).await;
    assert_eq!(s, 401);
    let (s, _, _) = call(&app, "GET", "/v1/health", None, "").await; // health is open and unmetered
    assert_eq!(s, 200);
    // burst 5 for alice (the two 401s above never spent tokens)
    for i in 0..5 {
        let (s, _, v) = call(&app, "POST", "/v1/act", Some(KEY), &body).await;
        assert_eq!(s, 200, "request {i}: {v}");
    }
    let (s, h, v) = call(&app, "POST", "/v1/act", Some(KEY), &body).await;
    assert_eq!((s, v["error"]["code"].as_str()), (429, Some("rate_limited")), "{v}");
    assert!(h.get("retry-after").is_some());
    // bob has his own bucket
    let (s, _, _) = call(&app, "POST", "/v1/act", Some(KEY2), &body).await;
    assert_eq!(s, 200);
    // after a refill (1 token/s) alice can ask again
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    let (s, _, _) = call(&app, "POST", "/v1/act", Some(KEY), &body).await;
    assert_eq!(s, 200);
}

/// Deep stacks: the `max_stack_bb` guard (150 bb) never lets the bot raise past its cap.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deep_stack_guard() {
    let core = need!(core_main());
    let deep = GenOpts { deep: true, bbs: &[100, 20, 50] };
    let mut guarded = 0;
    let mut raises = 0;
    for (i, sp) in gen_spots(12, 1500, deep, &|s| s.expect.raise.is_some()).iter().enumerate() {
        let req: ActRequest = serde_json::from_str(&no_search(sp.req.clone())).unwrap();
        let resp = core.decide(&req, format!("d{i}")).unwrap();
        let v = serde_json::to_value(&resp).unwrap();
        check_response(sp, &v);
        let cap = (150 * sp.expect.bb) as i64;
        if v["action"]["type"] == "raise" {
            raises += 1;
            let to = v["action"]["to"].as_i64().unwrap();
            assert!(sp.expect.hero_prior + to <= cap, "raise to {to} + prior {} exceeds cap {cap}: {v}", sp.expect.hero_prior);
        }
        if v["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("stack guard")) {
            guarded += 1;
        }
    }
    eprintln!("deep-stack: {raises} raises, {guarded} rewritten by the guard");
    assert!(raises > 30);
}

/// Real-chip semantics from TABLE_API: min-raise chain, "to" amounts, short all-in, etc.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn table_api_semantics() {
    let core = need!(core_main());
    let app = app(&core);
    // TABLE_API §12.2: open to 300 -> min re-raise to 500 -> re-raise to 1200 -> min next = 1900
    let r = json!({
        "small_blind": 50, "big_blind": 100, "button_seat": 3, "hero_seat": 2,
        "hole_cards": ["Ah", "Kd"],
        "players": (0..6).map(|s| json!({"seat": s, "stack_start": 10000})).collect::<Vec<_>>(),
        "history": [
            {"seat": 4, "type": "post_sb", "to": 50}, {"seat": 5, "type": "post_bb", "to": 100},
            {"seat": 0, "type": "fold"}, {"seat": 1, "type": "raise", "to": 300},
            {"seat": 2, "type": "raise", "to": 500}, {"seat": 3, "type": "raise", "to": 1200},
            {"seat": 4, "type": "fold"}, {"seat": 5, "type": "fold"}, {"seat": 1, "type": "fold"}
        ],
        "options": {"search": false, "seed": 1}
    });
    let (s, _, v) = call(&app, "POST", "/v1/act", Some(KEY), &r.to_string()).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["legal"]["call"]["to"], 1200);
    assert_eq!(v["legal"]["call"]["added"], 700);
    assert_eq!(v["legal"]["raise"]["min_to"], 1900);
    assert_eq!(v["legal"]["raise"]["max_to"], 10000);
    assert_eq!(v["street"], "preflop");
    // TABLE_API §12.3: short all-in does not reopen betting; the earlier raiser may only call or fold
    let r = json!({
        "small_blind": 50, "big_blind": 100, "button_seat": 0, "hero_seat": 1,
        "hole_cards": ["Qh", "Qd"],
        "players": [{"seat": 0, "stack_start": 10000}, {"seat": 1, "stack_start": 10000}, {"seat": 2, "stack_start": 1300}],
        "history": [
            {"seat": 1, "type": "post_sb"}, {"seat": 2, "type": "post_bb"},
            {"seat": 0, "type": "fold"}, {"seat": 1, "type": "raise", "to": 1000}, {"seat": 2, "type": "all_in"}
        ],
        "options": {"search": false}
    });
    let (s, _, v) = call(&app, "POST", "/v1/act", Some(KEY), &r.to_string()).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["legal"]["call"]["to"], 1300);
    assert!(v["legal"]["raise"].is_null(), "short all-in must not reopen the betting: {v}");
    // TABLE_API §6.6: a fold when checking is free is accepted as a check (and noted)
    let r = json!({
        "small_blind": 50, "big_blind": 100, "button_seat": 0, "hero_seat": 1,
        "hole_cards": ["7h", "2d"], "board": ["2c", "7d", "9h"],
        "players": [{"seat": 0, "stack_start": 5000}, {"seat": 1, "stack_start": 5000}, {"seat": 2, "stack_start": 5000}],
        "history": [{"seat": 0, "type": "call"}, {"seat": 1, "type": "call"}, {"seat": 2, "type": "fold"}],
        "options": {"search": false}
    });
    let (s, _, v) = call(&app, "POST", "/v1/act", Some(KEY), &r.to_string()).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["street"], "flop");
    assert!(v["notes"][0].as_str().unwrap().contains("treated as check"), "{v}");
    assert_eq!(v["legal"]["check"], true);
}

/// Hand already over -> 409, not a made-up action.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn finished_hand_is_conflict() {
    let core = need!(core_main());
    let app = app(&core);
    let r = json!({
        "small_blind": 50, "big_blind": 100, "button_seat": 0, "hero_seat": 2,
        "hole_cards": ["Ah", "Kd"],
        "players": (0..3).map(|s| json!({"seat": s, "stack_start": 10000})).collect::<Vec<_>>(),
        "history": [{"seat": 0, "type": "fold"}, {"seat": 1, "type": "fold"}]
    });
    // SB=1, BB=2, UTG=0: 0 folds, 1 folds -> BB (hero) wins uncontested
    let (s, _, v) = call(&app, "POST", "/v1/act", Some(KEY), &r.to_string()).await;
    assert_eq!((s, v["error"]["code"].as_str()), (409, Some("hand_over")), "{v}");
    let mut r2 = r.clone();
    r2["history"].as_array_mut().unwrap().push(json!({"seat": 2, "type": "check"}));
    let (s, _, v) = call(&app, "POST", "/v1/act", Some(KEY), &r2.to_string()).await;
    assert_eq!((s, v["error"]["code"].as_str()), (422, Some("action_after_hand_end")), "{v}");
}

/// Replay-only sanity on the generator itself: every generated request replays cleanly.
#[test]
fn generator_requests_replay_cleanly() {
    let mut rng = Rng::new(99);
    let mut n = 0;
    while n < 2000 {
        for sp in gen_hand(&mut rng, STD) {
            let req: ActRequest = serde_json::from_value(sp.req.clone()).unwrap();
            let h = Hand::replay(&req, None, 1).unwrap_or_else(|e| panic!("{e}: {}", sp.req));
            assert!(h.desync.is_none() || 100 % sp.expect.bb != 0, "desync {:?} for exact scale: {}", h.desync, sp.req);
            n += 1;
        }
    }
}

/// Public access: any arena player token works, vouched for once by the arena's `GET /v1/me` (cached),
/// with the public rate limit per player.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn arena_player_tokens() {
    use axum::http::HeaderMap as H;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    let mut cfg = need!(base_cfg());
    let calls = std::sync::Arc::new(AtomicUsize::new(0));
    let c2 = calls.clone();
    let fake = axum::Router::new().route(
        "/v1/me",
        axum::routing::get(move |h: H| {
            let c = c2.clone();
            async move {
                c.fetch_add(1, SeqCst);
                match h.get("authorization").and_then(|v| v.to_str().ok()) {
                    Some("Bearer sk_good") => (axum::http::StatusCode::OK, r#"{"player_id":"p_good","name":"good"}"#),
                    _ => (axum::http::StatusCode::UNAUTHORIZED, r#"{"error":{"code":"unauthorized"}}"#),
                }
            }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, fake).await.unwrap() });
    cfg.api_keys = vec![];
    cfg.arena_auth_url = Some(url);
    cfg.public_rps = 0.5;
    cfg.public_burst = 3.0;
    sharingan_serve::log::set_enabled(false);
    let core = std::sync::Arc::new(sharingan_serve::decide::Core::new(&cfg).expect("load tiny blueprint"));
    let app = app(&core);
    let sp = &gen_spots(12, 1, STD, &|_| true)[0];
    let body = no_search(sp.req.clone());
    for i in 0..3 {
        let (s, _, v) = call(&app, "POST", "/v1/act", Some("sk_good"), &body).await;
        assert_eq!(s, 200, "request {i}: {v}");
    }
    let (s, _, v) = call(&app, "POST", "/v1/act", Some("sk_good"), &body).await;
    assert_eq!((s, v["error"]["code"].as_str()), (429, Some("rate_limited")), "public burst is 3: {v}");
    assert_eq!(calls.load(SeqCst), 1, "the arena is asked once per token, then cached");
    for _ in 0..2 {
        let (s, _, v) = call(&app, "POST", "/v1/act", Some("sk_bad"), &body).await;
        assert_eq!((s, v["error"]["code"].as_str()), (401, Some("unauthorized")));
    }
    assert_eq!(calls.load(SeqCst), 2, "a rejected token is cached too");
    let (s, _, _) = call(&app, "POST", "/v1/act", None, &body).await;
    assert_eq!(s, 401);
}
