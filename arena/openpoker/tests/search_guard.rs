//! Search policy through the real session path on the offline mock table (no network).
//! Blueprint: a tiny 6-player toy blueprint trained in-process (`search::testkit`).

use bench::builtin::builtin;
use bench::policy::PolicyFactory;
use nlhe::abstraction::BetSizing;
use openpoker::offline;
use openpoker::searchpol::{finish, SearchOpts};
use openpoker::session::{HuMode, PolicySpec, Session, SessionConfig};
use openpoker::trace::MemSink;
use search::bp::Blueprint;
use search::testkit::train_blueprint;
use std::sync::{Arc, OnceLock};

fn bp6() -> Blueprint {
    static B: OnceLock<Blueprint> = OnceLock::new();
    B.get_or_init(|| train_blueprint(6, BetSizing::tiny(), Arc::new(bench::bp::CoarseBucketer), 40_000, 9)).clone()
}

fn run(opts: SearchOpts, hands: usize) -> (Session, MemSink, offline::OfflineResult, Arc<openpoker::guard::GuardedSearch>) {
    let g = Arc::new(finish(bp6(), &opts).unwrap());
    let sink = MemSink::default();
    let cfg = SessionConfig { buy_in: 2000, target_hands: hands, seed: 3, auto_rebuy: true, raw: false, sizing: "tiny".into(), buckets: "coarse".into(), max_stack_bb: 0.0 };
    let spec = PolicySpec { label: "search:test".into(), main: g.clone() as Arc<dyn PolicyFactory>, embed6: true, fallback: builtin("tag").unwrap(), hu: HuMode::Fallback };
    let mut s = Session::new(cfg, spec, Box::new(sink.clone()));
    let r = offline::run(&mut s, hands, 77);
    (s, sink, r, g)
}

fn bad(sink: &MemSink) -> Vec<serde_json::Value> {
    sink.of_kind("anomaly")
        .into_iter()
        .filter(|a| {
            matches!(
                a["what"].as_str().unwrap_or(""),
                "desync" | "replay_failed_will_rebuild" | "engine_server_mismatch" | "illegal_choice_replaced" | "profit_mismatch" | "your_turn_without_hand"
            )
        })
        .collect()
}

#[test]
fn offline_smoke_search_policy_plays_hundreds_of_hands() {
    let o = SearchOpts { threads: 2, budget_ms: 30_000, sets: "iters=300".into(), search_buckets: None };
    let (s, sink, r, g) = run(o, 300);
    assert_eq!(r.error, None);
    assert_eq!(r.hands, 300);
    assert_eq!(s.counters.hands_done, 300);
    assert!(bad(&sink).is_empty(), "{:#?}", &bad(&sink)[..bad(&sink).len().min(3)]);
    assert_eq!(s.counters.desync_hands, 0);
    assert_eq!(s.counters.fallbacks, 0);
    assert_eq!(s.counters.policy_fallbacks, 0, "no timeouts with a 30 s budget");
    assert!(g.searched() > 20, "search should have run, ran {}", g.searched());
    assert_eq!(g.timeouts() + g.panics(), 0);
    let dec = sink.of_kind("decision");
    assert!(dec.iter().all(|d| d["issues"].as_array().unwrap().is_empty()), "engine/server must agree");
}

#[test]
fn tiny_budget_falls_back_to_blueprint_cleanly() {
    // 1 ms hard budget with a search that would run for seconds (soft cap disabled via ms=).
    let o = SearchOpts { threads: 2, budget_ms: 1, sets: "iters=400000,ms=600000".into(), search_buckets: None };
    let (s, sink, r, g) = run(o, 60);
    assert_eq!(r.error, None, "session must keep answering every turn");
    assert_eq!(s.counters.hands_done, 60);
    assert!(bad(&sink).is_empty(), "{:#?}", &bad(&sink)[..bad(&sink).len().min(3)]);
    let dec = sink.of_kind("decision");
    let reasons: Vec<String> = dec.iter().filter_map(|d| d["fallback_reason"].as_str().map(str::to_string)).collect();
    assert!(!reasons.is_empty(), "expected timeouts to be traced");
    assert!(reasons.iter().any(|r| r.starts_with("search_timeout>1ms")), "{reasons:?}");
    assert_eq!(s.counters.policy_fallbacks as usize, reasons.len());
    assert!(g.timeouts() >= 1);
    // A fallback decision still carries a full, valid distribution and a sent action.
    for d in dec.iter().filter(|d| !d["fallback_reason"].is_null()) {
        assert!(!d["dist"].as_array().unwrap().is_empty());
        assert!(d["sent"].is_string());
    }
    // Each decision, fallback or not, returned promptly (hard budget + session overhead).
    assert!(dec.iter().all(|d| d["latency_us"].as_u64().unwrap() < 2_000_000), "decision exceeded 2 s");
}
