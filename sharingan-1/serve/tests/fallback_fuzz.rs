//! This API legality matrix always runs, including when no model fixture exists.
mod common;

use bench::bp::{sizing_by_name, CoarseBucketer, Source};
use common::{app, call, check_response, gen_spots, GenOpts, KEY};
use search::bp::Blueprint;
use serde_json::json;
use sharingan_serve::{config::Config, decide::{Core, Meta}};
use std::sync::Arc;

struct Calls;
impl Source for Calls {
    fn probs(&self, _: u64, _: u32, out: &mut [f32]) -> bool {
        out.fill(0.0);
        let call = usize::from(out.len() > 1);
        out[call] = 1.0;
        true
    }
}

#[tokio::test]
async fn generated_real_chip_histories_stay_legal_without_a_model_fixture() {
    sharingan_serve::log::set_enabled(false);
    let bp = Blueprint::from_source(
        Arc::new(Calls), Arc::new(CoarseBucketer), sizing_by_name("medium").unwrap(),
    );
    let cfg = Config {
        search: false, search_threads: 1,
        api_keys: vec![("test".into(), KEY.into())], rate_rps: 100_000.0, rate_burst: 100_000.0,
        ..Config::default()
    };
    let meta = Meta {
        path: "synthetic-call-source".into(), infosets: 0, file_bytes: 0,
        src_infosets: 0, used_snapshots: false,
    };
    let core = Arc::new(Core::with_blueprint(&cfg, bp, meta).unwrap());
    let app = app(&core);
    let mut streets = [0; 4];
    let mut players = [0; 7];
    let mut fallbacks = 0;
    for deep in [false, true] {
        let spots = gen_spots(2026110512 + u64::from(deep), 2000,
            GenOpts { deep, bbs: &[2, 10, 20, 25, 30, 50, 100, 200, 1000, 2000, 10000] }, &|_| true);
        assert_eq!(spots.len(), 2000);
        for spot in spots {
            streets[spot.expect.street] += 1;
            players[spot.expect.n] += 1;
            let mut req = spot.req.clone();
            req["options"] = json!({"search": false, "seed": 7, "include_probs": true});
            let (status, _, response) = call(&app, "POST", "/v1/act", Some(KEY), &req.to_string()).await;
            assert_eq!(status, 200, "{req} -> {response}");
            check_response(&spot, &response);
            let sum: f64 = response["probs"].as_array().unwrap().iter()
                .map(|p| p["p"].as_f64().unwrap()).sum();
            assert!((sum - 1.0).abs() < 2e-3, "{response}");
            assert!(response["probs"].as_array().unwrap().iter().any(|p|
                p["type"] == response["action"]["type"] &&
                p["to"] == response["action"]["to"] && p["p"].as_f64().unwrap() > 0.0),
                "returned action has no probability: {req} -> {response}");
            if response["source"] == "fallback" { fallbacks += 1; }
        }
    }
    assert!(streets.iter().all(|&n| n > 100), "streets: {streets:?}");
    assert!((2..=6).all(|n| players[n] > 100), "players: {players:?}");
    // Random histories need not hit the very small scaled-terminal window. Require
    // that path explicitly instead of treating a zero generated count as coverage.
    let mut history = vec![json!({"seat": 2, "type": "raise", "to": 99999})];
    for seat in [3, 4, 5, 0, 1] {
        history.push(json!({"seat": seat, "type": "call"}));
    }
    history.push(json!({"seat": 0, "type": "all_in", "to": 1}));
    let req = json!({
        "small_blind": 500, "big_blind": 1000, "button_seat": 5, "hero_seat": 1,
        "hole_cards": ["Ah", "Kh"], "board": ["Qh", "Jh", "Th"],
        "players": (0..6).map(|seat| json!({"seat": seat, "stack_start": 100000})).collect::<Vec<_>>(),
        "history": history, "options": {"search": false, "seed": 7, "include_probs": true}
    });
    let (status, _, response) = call(&app, "POST", "/v1/act", Some(KEY), &req.to_string()).await;
    assert_eq!(status, 200);
    assert_eq!(response["source"], "fallback");
    assert_eq!(response["legal"]["call"]["added"], 1);
    assert!(matches!(response["action"]["type"].as_str(), Some("fold" | "call")));
    let p = response["probs"].as_array().unwrap();
    assert_eq!(p.len(), 1);
    assert_eq!(p[0]["type"], response["action"]["type"]);
    assert_eq!(p[0]["to"], response["action"]["to"]);
    assert_eq!(p[0]["p"], 1.0);
    eprintln!("4000 generated HTTP cases: streets {streets:?}, players {players:?}, generated outer fallbacks {fallbacks}; one explicit scaled-terminal HTTP control passed");
}
