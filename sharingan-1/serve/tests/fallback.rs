//! Always-run HTTP regressions: no external blueprint or bucket files required.
use axum::{body::Body, http::Request};
use bench::bp::{sizing_by_name, CoarseBucketer, Source};
use http_body_util::BodyExt;
use search::bp::Blueprint;
use serde_json::{json, Value};
use sharingan_serve::{config::Config, decide::{Core, Meta}, server::{router, AppState}};
use std::sync::Arc;
use tower::ServiceExt;

struct Calls;
impl Source for Calls {
    fn probs(&self, _: u64, _: u32, out: &mut [f32]) -> bool {
        out.fill(0.0);
        let call = usize::from(out.len() > 1);
        out[call] = 1.0;
        true
    }
}

fn app() -> axum::Router {
    let bp = Blueprint::from_source(
        Arc::new(Calls), Arc::new(CoarseBucketer), sizing_by_name("medium").unwrap(),
    );
    let cfg = Config {
        search: false, search_threads: 1,
        api_keys: vec![("test".into(), "fallback-test-key".into())],
        ..Config::default()
    };
    let meta = Meta {
        path: "synthetic-call-source".into(), infosets: 0, file_bytes: 0,
        src_infosets: 0, used_snapshots: false,
    };
    router(AppState::new(Arc::new(Core::with_blueprint(&cfg, bp, meta).unwrap())))
}

fn request(bb: i64, hero: usize) -> Value {
    let stack = 100 * bb;
    let mut history = vec![json!({"seat": 2, "type": "raise", "to": stack - 1})];
    for seat in [3, 4, 5, 0, 1] {
        history.push(json!({"seat": seat, "type": "call"}));
    }
    history.push(json!({"seat": 0, "type": "all_in", "to": 1}));
    for seat in 1..hero {
        history.push(json!({"seat": seat, "type": "call"}));
    }
    json!({
        "small_blind": bb / 2, "big_blind": bb, "button_seat": 5, "hero_seat": hero,
        "hole_cards": ["Ah", "Kh"], "board": ["Qh", "Jh", "Th"],
        "players": (0..6).map(|seat| json!({"seat": seat, "stack_start": stack})).collect::<Vec<_>>(),
        "history": history,
        "options": {"search": false, "seed": 1, "include_probs": true}
    })
}

async fn post(app: &axum::Router, path: &str, body: Value) -> Value {
    let request = Request::post(path)
        .header("content-type", "application/json")
        .header("authorization", "Bearer fallback-test-key")
        .body(Body::from(body.to_string())).unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), 200);
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

fn check(response: &Value, bb: i64) {
    assert_eq!(response["action"]["type"], "call", "{response}");
    assert_eq!(response["action"]["to"], 1);
    assert_eq!(response["action"]["added"], 1);
    assert_eq!(response["legal"]["call"]["added"], 1);
    if bb == 100 {
        assert_eq!(response["source"], "blueprint");
    } else {
        assert_eq!(response["source"], "fallback");
        assert!(response["fallback_reason"].as_str().unwrap().starts_with("desync: "));
        assert_eq!(response["probs"], json!([{"type": "call", "to": 1, "p": 1.0}]));
    }
}

#[tokio::test]
async fn scaled_terminal_nuts_call_through_http_and_batch() {
    let app = app();
    let mut requests = Vec::new();
    let mut blinds = Vec::new();
    for bb in [100, 1_000, 2_000, 10_000, 100_000] {
        for hero in 1..6 {
            let req = request(bb, hero);
            check(&post(&app, "/v1/act", req.clone()).await, bb);
            requests.push(req);
            blinds.push(bb);
        }
    }
    let batch = post(&app, "/v1/act/batch", json!({"requests": requests})).await;
    let results = batch["results"].as_array().unwrap();
    assert_eq!(results.len(), blinds.len());
    for (result, bb) in results.iter().zip(blinds) {
        assert_eq!(result["ok"], true, "{result}");
        check(&result["response"], bb);
    }
}
