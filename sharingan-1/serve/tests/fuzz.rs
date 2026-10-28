//! Illegal / inconsistent histories must be rejected with a clear 4xx, never a 5xx or a made-up action.

mod common;

use cards::Rng;
use common::*;
use serde_json::{json, Value};
use sharingan_serve::api::ActRequest;
use sharingan_serve::replay::Hand;

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

fn with_opts(mut v: Value) -> String {
    v["options"] = json!({"search": false, "seed": 3});
    v.to_string()
}

async fn expect_err(app: &axum::Router, req: &Value, status: u16, code: &str, what: &str) {
    let (s, _, v) = call(app, "POST", "/v1/act", Some(KEY), &with_opts(req.clone())).await;
    assert_eq!((s, v["error"]["code"].as_str()), (status, Some(code)), "{what}: {req} -> {v}");
    assert!(v["error"]["message"].as_str().is_some_and(|m| !m.is_empty()), "{what}: error needs a message");
}

/// Targeted mutations of valid requests, each with its exact expected error code.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn targeted_illegal_histories() {
    let core = need!(core_main());
    let app = app(&core);
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    let mut rng = Rng::new(77);
    for sp in gen_spots(21, 600, STD, &|_| true) {
        let r = sp.req.clone();
        let hist = r["history"].as_array().unwrap().clone();
        let street = sp.expect.street;
        let hero = r["hero_seat"].as_i64().unwrap();
        let mut bump = |k: &'static str| *counts.entry(k).or_default() += 1;

        // a raise one chip over its maximum / one under its minimum
        if let Some(&(i, lo, hi)) = sp.raise_events.first() {
            let mut m = r.clone();
            m["history"][i]["type"] = json!("raise");
            m["history"][i]["to"] = json!(hi + 1);
            expect_err(&app, &m, 422, "bad_amount", "raise over max").await;
            if lo > 0 {
                let mut m = r.clone();
                m["history"][i]["type"] = json!("raise");
                m["history"][i]["to"] = json!(lo - 1);
                // lo-1 may equal the actor's max when lo was clamped to a short all-in, so only assert when not
                if lo - 1 < hi && lo < hi {
                    expect_err(&app, &m, 422, "bad_amount", "raise under min").await;
                }
            }
            let mut m = r.clone();
            m["history"][i].as_object_mut().unwrap().remove("to");
            m["history"][i]["type"] = json!("raise");
            expect_err(&app, &m, 422, "missing_amount", "raise without to").await;
            bump("raise_amount");
        }
        // check while facing a bet
        if let Some(&i) = sp.call_events.first() {
            let mut m = r.clone();
            m["history"][i]["type"] = json!("check");
            m["history"][i].as_object_mut().unwrap().remove("to");
            expect_err(&app, &m, 422, "illegal_action", "check facing a bet").await;
            bump("check_facing_bet");
        }
        // the last event removed: the previous actor is not the hero -> not_hero_turn (or the hero acted... never twice)
        if hist.iter().any(|e| !matches!(e["type"].as_str(), Some("post_sb" | "post_bb"))) {
            let mut m = r.clone();
            m["history"].as_array_mut().unwrap().pop();
            let (s, _, v) = call(&app, "POST", "/v1/act", Some(KEY), &with_opts(m.clone())).await;
            assert!(s == 422 || s == 200, "{m} -> {v}");
            bump("drop_last");
        }
        // a different seat acts first
        {
            let mut m = r.clone();
            let arr = m["history"].as_array_mut().unwrap();
            if let Some(i) = arr.iter().position(|e| !matches!(e["type"].as_str(), Some("post_sb" | "post_bb"))) {
                let seat = arr[i]["seat"].as_i64().unwrap();
                arr[i]["seat"] = json!(if seat == 9 { 8 } else { seat + 1 });
                let (s, _, v) = call(&app, "POST", "/v1/act", Some(KEY), &with_opts(m.clone())).await;
                // wrong seat: out_of_turn, or unknown_seat when it is not dealt in
                assert_eq!(s, 422, "{m} -> {v}");
                assert!(matches!(v["error"]["code"].as_str(), Some("out_of_turn" | "unknown_seat")), "{v}");
                bump("wrong_seat");
            }
        }
        // street label disagreeing with the engine
        if let Some(i) = hist.iter().position(|e| e.get("street").is_some() && !matches!(e["type"].as_str(), Some("post_sb" | "post_bb"))) {
            let mut m = r.clone();
            let cur = m["history"][i]["street"].as_str().unwrap().to_string();
            m["history"][i]["street"] = json!(if cur == "river" { "flop" } else { "river" });
            expect_err(&app, &m, 422, "street_mismatch", "street label").await;
            bump("street_label");
        }
        // board problems
        if street > 0 {
            let mut m = r.clone();
            m["board"].as_array_mut().unwrap().pop();
            expect_err(&app, &m, 422, "board_mismatch", "board short").await;
            let mut m = r.clone();
            m["board"] = json!([]);
            expect_err(&app, &m, 422, "board_mismatch", "board missing").await;
            let mut m = r.clone();
            let b = m["board"].as_array().unwrap()[0].clone();
            m["hole_cards"][0] = b;
            expect_err(&app, &m, 422, "duplicate_card", "hole card on the board").await;
            bump("board");
        }
        {
            let mut m = r.clone();
            m["board"].as_array_mut().unwrap().push(json!("2c"));
            let (s, _, v) = call(&app, "POST", "/v1/act", Some(KEY), &with_opts(m.clone())).await;
            assert_eq!(s, 422, "extra board card: {m} -> {v}");
            assert!(matches!(v["error"]["code"].as_str(), Some("board_mismatch" | "duplicate_card")));
        }
        // cards
        {
            let mut m = r.clone();
            m["hole_cards"][1] = m["hole_cards"][0].clone();
            expect_err(&app, &m, 422, "duplicate_card", "same hole card twice").await;
            let mut m = r.clone();
            m["hole_cards"][0] = json!("Zz");
            expect_err(&app, &m, 422, "invalid_card", "bad card").await;
            let mut m = r.clone();
            m["hole_cards"] = json!(["Ah"]);
            expect_err(&app, &m, 422, "invalid_card", "one hole card").await;
        }
        // players / seats / blinds
        {
            let mut m = r.clone();
            m["hero_seat"] = json!(hero + 100);
            expect_err(&app, &m, 422, "hero_not_in_hand", "hero out of range").await;
            let mut m = r.clone();
            let p0 = m["players"][0].clone();
            m["players"].as_array_mut().unwrap().push(p0);
            expect_err(&app, &m, 422, "duplicate_seat", "duplicate seat").await;
            let mut m = r.clone();
            m["players"][0]["stack_start"] = json!(0);
            expect_err(&app, &m, 422, "bad_stack", "zero stack").await;
            let mut m = r.clone();
            m["players"][0]["stack_start"] = json!(-5);
            expect_err(&app, &m, 422, "bad_stack", "negative stack").await;
            let mut m = r.clone();
            m["small_blind"] = json!(0);
            expect_err(&app, &m, 422, "bad_blinds", "zero blind").await;
            let mut m = r.clone();
            m["small_blind"] = json!(m["big_blind"].as_i64().unwrap() + 1);
            expect_err(&app, &m, 422, "bad_blinds", "sb > bb").await;
            let mut m = r.clone();
            m["button_seat"] = json!(-1);
            expect_err(&app, &m, 422, "bad_seat", "negative button").await;
            let mut m = r.clone();
            m["history"].as_array_mut().unwrap().push(json!({"seat": 0, "type": "teleport"}));
            let (s, _, v) = call(&app, "POST", "/v1/act", Some(KEY), &with_opts(m.clone())).await;
            assert_eq!(s, 422, "{v}");
            bump("players");
        }
        // too many / too few players
        if rng.below(10) == 0 {
            let mut m = r.clone();
            m["players"] = json!((0..7).map(|s| json!({"seat": s, "stack_start": 1000})).collect::<Vec<_>>());
            m["hero_seat"] = json!(0);
            expect_err(&app, &m, 422, "bad_player_count", "seven players").await;
            let mut m = r.clone();
            m["players"] = json!([{"seat": hero, "stack_start": 1000}]);
            expect_err(&app, &m, 422, "bad_player_count", "one player").await;
        }
    }
    eprintln!("mutation classes exercised: {counts:?}");
    for k in ["raise_amount", "check_facing_bet", "drop_last", "wrong_seat", "street_label", "board", "players"] {
        assert!(counts.get(k).copied().unwrap_or(0) >= 20, "mutation class {k} barely exercised: {counts:?}");
    }
}

/// Malformed bodies: 400 bad_request (never a panic or 5xx).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn malformed_bodies() {
    let core = need!(core_main());
    let app = app(&core);
    let sp = &gen_spots(22, 1, STD, &|_| true)[0];
    let ok = sp.req.clone();
    let mut bodies: Vec<String> = vec![
        "".into(),
        "null".into(),
        "[]".into(),
        "{".into(),
        "{}".into(),
        r#"{"small_blind":"50"}"#.into(),
        "\u{0}\u{1}\u{2}".into(),
    ];
    for k in ["small_blind", "big_blind", "button_seat", "hero_seat", "hole_cards", "players"] {
        let mut m = ok.clone();
        m.as_object_mut().unwrap().remove(k);
        bodies.push(m.to_string());
    }
    let mut m = ok.clone();
    m["small_blind"] = json!(50.5); // chips are integers
    bodies.push(m.to_string());
    let mut m = ok.clone();
    m["big_blind"] = json!("100");
    bodies.push(m.to_string());
    let mut m = ok.clone();
    m["history"] = json!("nope");
    bodies.push(m.to_string());
    let mut m = ok.clone();
    m["history"] = json!([{"seat": "a", "type": 5}]);
    bodies.push(m.to_string());
    let mut m = ok.clone();
    m["players"] = json!([{"seat": 1}]);
    bodies.push(m.to_string());
    for b in &bodies {
        let (s, _, v) = call(&app, "POST", "/v1/act", Some(KEY), b).await;
        assert_eq!((s, v["error"]["code"].as_str()), (400, Some("bad_request")), "body {b:?} -> {v}");
    }
    // huge numbers are rejected as semantic errors, not overflows
    for (k, val) in [("big_blind", json!(i64::MAX)), ("small_blind", json!(i64::MAX))] {
        let mut m = ok.clone();
        m[k] = val;
        let (s, _, v) = call(&app, "POST", "/v1/act", Some(KEY), &with_opts(m)).await;
        assert!(s == 422 || s == 400, "{v}");
    }
    let mut m = ok.clone();
    m["players"][0]["stack_start"] = json!(i64::MAX);
    let (s, _, _) = call(&app, "POST", "/v1/act", Some(KEY), &with_opts(m)).await;
    assert_eq!(s, 422);
    // zero time budget
    let mut m = ok.clone();
    m["options"] = json!({"time_budget_ms": 0});
    let (s, _, v) = call(&app, "POST", "/v1/act", Some(KEY), &m.to_string()).await;
    assert_eq!((s, v["error"]["code"].as_str()), (422, Some("bad_budget")));
    // wrong method
    let (s, _, _) = call(&app, "GET", "/v1/act", Some(KEY), "").await;
    assert_eq!(s, 405);
}

/// Random structural corruption of valid requests: the answer is always a clean 4xx or a legal action.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn random_corruption_never_5xx_and_200s_are_legal() {
    let core = need!(core_main());
    let app = app(&core);
    let mut rng = Rng::new(31337);
    let (mut ok, mut rejected) = (0, 0);
    for (i, sp) in gen_spots(23, 2500, STD, &|_| true).into_iter().enumerate() {
        let mut m = sp.req.clone();
        for _ in 0..1 + rng.below(3) {
            let hist_len = m["history"].as_array().unwrap().len();
            match rng.below(10) {
                0 if hist_len > 0 => {
                    let j = rng.below(hist_len as u32) as usize;
                    m["history"].as_array_mut().unwrap().remove(j);
                }
                1 if hist_len > 0 => {
                    let j = rng.below(hist_len as u32) as usize;
                    let e = m["history"][j].clone();
                    m["history"].as_array_mut().unwrap().insert(j, e);
                }
                2 if hist_len > 1 => {
                    let (a, b) = (rng.below(hist_len as u32) as usize, rng.below(hist_len as u32) as usize);
                    m["history"].as_array_mut().unwrap().swap(a, b);
                }
                3 if hist_len > 0 => {
                    let j = rng.below(hist_len as u32) as usize;
                    m["history"][j]["seat"] = json!(rng.below(10) as i64 - 1);
                }
                4 if hist_len > 0 => {
                    let j = rng.below(hist_len as u32) as usize;
                    let v = [json!(-5), json!(0), json!(1), json!(7), json!(1_000_000_000), json!(i64::MAX), json!(null)][rng.below(7) as usize].clone();
                    m["history"][j]["to"] = v;
                }
                5 if hist_len > 0 => {
                    let j = rng.below(hist_len as u32) as usize;
                    let t = ["fold", "check", "call", "bet", "raise", "all_in", "post_sb", "post_bb", "bogus"][rng.below(9) as usize];
                    m["history"][j]["type"] = json!(t);
                }
                6 => {
                    let b = m["board"].as_array_mut().unwrap();
                    if !b.is_empty() && rng.below(2) == 0 {
                        b.pop();
                    } else {
                        b.push(json!(["2c", "Ah", "Kd", "9s", "??"][rng.below(5) as usize]));
                    }
                }
                7 => m["button_seat"] = json!(rng.below(12) as i64 - 1),
                8 => m["hero_seat"] = json!(rng.below(10)),
                _ => {
                    let p = m["players"].as_array_mut().unwrap();
                    let j = rng.below(p.len() as u32) as usize;
                    p[j]["stack_start"] = json!([0, 1, 50, 99, 100_000_000_000i64, 12345][rng.below(6) as usize]);
                }
            }
        }
        let (s, _, v) = call(&app, "POST", "/v1/act", Some(KEY), &with_opts(m.clone())).await;
        assert!(s < 500, "request #{i} caused {s}: {m} -> {v}");
        if s == 200 {
            ok += 1;
            // the mutated request must still replay cleanly and the answer must be legal in it
            let req: ActRequest = serde_json::from_value(m.clone()).unwrap();
            let h = Hand::replay(&req, None, 1).unwrap_or_else(|e| panic!("200 but replay fails ({e}): {m}"));
            let a = &v["action"];
            let l = h.legal_out();
            match a["type"].as_str().unwrap() {
                "fold" => assert!(l.fold, "{m} -> {v}"),
                "check" => assert!(l.check, "{m} -> {v}"),
                "call" => assert!(l.call.is_some(), "{m} -> {v}"),
                "raise" => {
                    let r = l.raise.unwrap_or_else(|| panic!("{m} -> {v}"));
                    let to = a["to"].as_i64().unwrap();
                    assert!(to >= r.min_to && to <= r.max_to, "{m} -> {v}");
                }
                k => panic!("{k}"),
            }
        } else {
            rejected += 1;
            assert!(v["error"]["code"].is_string() && v["error"]["message"].is_string(), "{v}");
        }
    }
    eprintln!("random corruption: {ok} still-valid answers, {rejected} rejected");
    assert!(rejected > 1000 && ok > 50);
}
