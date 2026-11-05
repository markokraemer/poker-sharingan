//! Probability metadata must describe actions after the real-chip stack guard.
use bench::bp::{sizing_by_name, CoarseBucketer, Source};
use search::bp::Blueprint;
use serde_json::json;
use sharingan_serve::{api::ActRequest, config::Config, decide::{Core, Meta}};
use std::sync::Arc;

struct Shoves;
impl Source for Shoves {
    fn probs(&self, _: u64, _: u32, out: &mut [f32]) -> bool {
        out.fill(0.0);
        *out.last_mut().unwrap() = 1.0;
        true
    }
}

fn core(cap: f64) -> Core {
    core_with_source(cap, Arc::new(Shoves))
}

fn core_with_source(cap: f64, source: Arc<dyn Source>) -> Core {
    let bp = Blueprint::from_source(
        source, Arc::new(CoarseBucketer), sizing_by_name("medium").unwrap(),
    );
    let cfg = Config { search: false, search_threads: 1, max_stack_bb: cap, ..Config::default() };
    let meta = Meta { path: "synthetic-shove-source".into(), infosets: 0, file_bytes: 0,
                      src_infosets: 0, used_snapshots: false };
    Core::with_blueprint(&cfg, bp, meta).unwrap()
}

#[test]
fn capped_probabilities_describe_the_actual_real_chip_action() {
    let core = core(150.0);
    let mut mismatches = 0;
    let mut records = Vec::new();
    for case in 0..512 {
        let bb = [20, 50, 100, 200, 1000, 2000][case % 6];
        let button = case % 6;
        let stack = (200 + case % 800) * bb;
        let req: ActRequest = serde_json::from_value(json!({
            "small_blind": bb / 2, "big_blind": bb, "button_seat": button,
            "hero_seat": (button + 3) % 6, "hole_cards": ["Ah", "Kd"],
            "players": (0..6).map(|seat| json!({"seat": seat, "stack_start": stack})).collect::<Vec<_>>(),
            "history": [], "options": {"search": false, "seed": case, "include_probs": true}
        })).unwrap();
        let response = core.decide(&req, format!("cap-{case}")).unwrap();
        assert_eq!(response.source, "blueprint");
        assert_eq!(response.action.kind, "raise");
        assert_eq!(response.action.to, Some((150 * bb) as i64));
        let probabilities: Vec<_> = response.probs.as_ref().unwrap().iter().filter(|p| p.p > 0.0).collect();
        assert_eq!(probabilities.len(), 1);
        assert_eq!(probabilities[0].kind, response.action.kind);
        mismatches += usize::from(probabilities[0].to != response.action.to);
        assert_eq!(probabilities[0].p, 1.0);
        let mut record = serde_json::to_value(&response).unwrap();
        record.as_object_mut().unwrap().remove("timing");
        records.push(record);
    }
    if let Ok(path) = std::env::var("CAP_PROBS_OUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&json!({
            "cases": records.len(), "mismatches": mismatches, "responses": records
        })).unwrap()).unwrap();
    }
    assert_eq!(mismatches, 0, "reported probabilities disagree with actual capped actions");
}

#[test]
fn uncapped_probabilities_keep_the_original_shove() {
    let core = core(0.0);
    let req: ActRequest = serde_json::from_value(json!({
        "small_blind": 50, "big_blind": 100, "button_seat": 5, "hero_seat": 2,
        "hole_cards": ["Ah", "Kd"],
        "players": (0..6).map(|seat| json!({"seat": seat, "stack_start": 100000})).collect::<Vec<_>>(),
        "history": [], "options": {"search": false, "seed": 7, "include_probs": true}
    })).unwrap();
    let response = core.decide(&req, "no-cap".into()).unwrap();
    assert_eq!(response.action.to, Some(100000));
    let probabilities: Vec<_> = response.probs.as_ref().unwrap().iter().filter(|p| p.p > 0.0).collect();
    assert_eq!(probabilities.len(), 1);
    assert_eq!(probabilities[0].to, Some(100000));
    assert_eq!(probabilities[0].p, 1.0);
}

#[test]
fn several_raises_capped_to_one_action_merge_their_probabilities() {
    struct TwoRaises;
    impl Source for TwoRaises {
        fn probs(&self, _: u64, _: u32, out: &mut [f32]) -> bool {
            out.fill(0.0);
            let n = out.len();
            out[n - 1] = 3.0;
            out[n - 2] = 1.0;
            true
        }
    }
    let core = core_with_source(3.0, Arc::new(TwoRaises));
    let req: ActRequest = serde_json::from_value(json!({
        "small_blind": 50, "big_blind": 100, "button_seat": 5, "hero_seat": 2,
        "hole_cards": ["Ah", "Kd"],
        "players": (0..6).map(|seat| json!({"seat": seat, "stack_start": 100000})).collect::<Vec<_>>(),
        "history": [], "options": {"search": false, "seed": 7, "include_probs": true}
    })).unwrap();
    let response = core.decide(&req, "coalesced-cap".into()).unwrap();
    assert_eq!(response.action.to, Some(300));
    let positive: Vec<_> = response.probs.as_ref().unwrap().iter().filter(|p| p.p > 0.0).collect();
    assert_eq!(positive.len(), 1);
    assert_eq!(positive[0].kind, "raise");
    assert_eq!(positive[0].to, Some(300));
    assert_eq!(positive[0].p, 1.0);
}

#[test]
fn deterministic_fallback_reports_the_action_it_returns() {
    let core = core(150.0);
    let mut history = vec![json!({"seat": 2, "type": "raise", "to": 99999})];
    for seat in [3, 4, 5, 0, 1] {
        history.push(json!({"seat": seat, "type": "call"}));
    }
    history.push(json!({"seat": 0, "type": "all_in", "to": 1}));
    let req: ActRequest = serde_json::from_value(json!({
        "small_blind": 500, "big_blind": 1000, "button_seat": 5, "hero_seat": 1,
        "hole_cards": ["Ah", "Kh"], "board": ["Qh", "Jh", "Th"],
        "players": (0..6).map(|seat| json!({"seat": seat, "stack_start": 100000})).collect::<Vec<_>>(),
        "history": history, "options": {"search": false, "seed": 7, "include_probs": true}
    })).unwrap();
    let response = core.decide(&req, "fallback-probs".into()).unwrap();
    assert_eq!(response.source, "fallback");
    let probabilities = response.probs.as_ref().unwrap();
    assert_eq!(probabilities.len(), 1);
    assert_eq!(probabilities[0].kind, response.action.kind);
    assert_eq!(probabilities[0].to, response.action.to);
    assert_eq!(probabilities[0].p, 1.0);
}
