//! End-to-end checks of the evaluation pipeline on a toy 6-max blueprint (tiny sizing, coarse
//! buckets): checkpoint -> compact -> h2h / exploit-compact.

use bench::bp::{train_toy, CoarseBucketer};
use bench::compact::CompactPolicy;
use bench::policy::PolicyFactory;
use blueprint::compact::{compact_checkpoint, CompactOptions};
use nlhe::abstraction::{abstract_actions, ActionList, BetSizing, History};
use nlhe::{GameConfig, State};
use pluribus::h2h::{h2h, print_summary, H2hConfig};
use std::path::PathBuf;
use std::sync::Arc;

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("pl_eval_{}_{}", std::process::id(), name))
}

fn make(players: usize, iters: u64, seed: u64, tag: &str) -> (PathBuf, PathBuf) {
    let bp = train_toy(players, iters, seed);
    let ck = tmp(&format!("{tag}.bin"));
    blueprint::io::save(bp.store(), &ck).unwrap();
    let out = tmp(&format!("{tag}.plcp"));
    let opt = CompactOptions { threads: 4, min_visits: 0, chunk: 1 << 14, group_slots: 1 << 24, part_bytes: 4 << 20, tmp_dir: None };
    let st = compact_checkpoint(&ck, None, &out, &opt, &|_| {}).unwrap();
    eprintln!("{tag}: {} infosets -> {} bytes ({:.2} B/infoset)", st.infosets_out, st.bytes_out, st.bytes_out as f64 / st.infosets_out as f64);
    (ck, out)
}

fn pol(path: &PathBuf, name: &str) -> Arc<dyn PolicyFactory> {
    let p = CompactPolicy::open(path, Arc::new(CoarseBucketer), BetSizing::tiny()).unwrap();
    struct Named(CompactPolicy, String);
    impl PolicyFactory for Named {
        fn name(&self) -> String {
            self.1.clone()
        }
        fn new_player(&self, seat: usize, cfg: &nlhe::GameConfig, seed: u64) -> Box<dyn bench::policy::Player> {
            self.0.new_player(seat, cfg, seed)
        }
    }
    Arc::new(Named(p, name.to_string()))
}

fn known_payoff_pair() -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    // Every aggressive seat shoves/calls; every other seat folds to a bet.
    // Enumerate all public histories in the tested seatings and store the same
    // action for all 169 preflop classes. These profiles end before postflop
    // decisions. This tests persisted/compacted strategies, not CFR convergence.
    let stores = [blueprint::store::Store::new(1 << 20), blueprint::store::Store::new(1 << 20)];
    let sizing = BetSizing::tiny();
    for mask in 0..64u32 {
        if ![1, 3, 5].contains(&mask.count_ones()) { continue; }
        let mut state = State::new(&GameConfig::new(6));
        let mut history = History::new();
        while !state.is_terminal() {
            assert_eq!(state.street(), nlhe::Street::Preflop);
            let aggressive = mask & (1 << state.to_act().unwrap()) != 0;
            let mut list = ActionList::new();
            abstract_actions(&state, &sizing, &mut list);
            let chosen = if aggressive { list.len() - 1 } else { 0 };
            for bucket in 0..169 {
                let slot = stores[usize::from(aggressive)].infoset(history.key(), bucket, list.len(), 169, true);
                stores[usize::from(aggressive)].add_regret(slot, chosen, 1.0);
                stores[usize::from(aggressive)].bump_avg(slot, chosen);
            }
            history.push(chosen);
            state.apply(list.get(chosen));
        }
    }
    let save = |store: &blueprint::store::Store, tag: &str| {
        let checkpoint = tmp(&format!("{tag}.bin"));
        let compact = tmp(&format!("{tag}.plcp"));
        blueprint::io::save(store, &checkpoint).unwrap();
        let options = CompactOptions { threads: 4, min_visits: 0, chunk: 1 << 14, group_slots: 1 << 24, part_bytes: 4 << 20, tmp_dir: None };
        compact_checkpoint(&checkpoint, None, &compact, &options, &|_| {}).unwrap();
        (checkpoint, compact)
    };
    let (strong_checkpoint, strong) = save(&stores[1], "known_aggressive");
    let (weak_checkpoint, weak) = save(&stores[0], "known_fold");
    (strong_checkpoint, strong, weak_checkpoint, weak)
}

#[test]
fn h2h_self_match_is_zero() {
    let (ck, out) = make(6, 60_000, 5, "self");
    // Two independent factory instances over the same file.
    let cfg = H2hConfig { players: 6, deals: 3000, seed: 7, threads: 4 };
    let rep = h2h(pol(&out, "A"), pol(&out, "B"), &cfg);
    print_summary(&rep);
    for r in rep["results"].as_array().unwrap() {
        let (m, ci) = (r["mbb"]["mbb"].as_f64().unwrap(), r["mbb"]["ci95"].as_f64().unwrap());
        assert!(m.abs() <= 3.0 * ci.max(1.0), "{}: {m} +- {ci}", r["format"]);
    }
    let _ = std::fs::remove_file(ck);
    let _ = std::fs::remove_file(out);
}

#[test]
fn h2h_detects_a_clear_difference() {
    // All pots go to aggressive seats; their group's gain is exactly the
    // folding seats' posted blinds, regardless of the cards or training races.
    let (c1, strong, c2, weak) = known_payoff_pair();
    let cfg = H2hConfig { players: 6, deals: 3000, seed: 9, threads: 4 };
    let rep = h2h(pol(&strong, "strong"), pol(&weak, "weak"), &cfg);
    print_summary(&rep);
    let first = &rep["results"][0]["mbb"];
    let (m, ci) = (first["mbb"].as_f64().unwrap(), first["ci95"].as_f64().unwrap());
    assert!(m - ci > 0.0, "known winning profile must have a positive lower bound: {m} +- {ci}");
    for (result, expected) in rep["results"].as_array().unwrap().iter().zip([250.0, 1250.0, -250.0]) {
        let raw = &result["raw_mbb"];
        let actual = raw["mbb"].as_f64().unwrap();
        assert!((actual - expected).abs() < 1e-8, "{}: {actual} instead of {expected}", result["format"]);
        assert!(raw["ci95"].as_f64().unwrap() < 1e-6, "known group payoff is card-independent");
    }
    for p in [c1, strong, c2, weak] {
        let _ = std::fs::remove_file(p);
    }
}

#[test]
fn exploit_compact_agrees_with_exploit_on_store() {
    let (ck, out) = make(2, 40_000, 4, "exp");
    let run = |compact: bool, target: &PathBuf| {
        let a: Vec<String> = ["--target", target.to_str().unwrap(), "--sizing", "tiny", "--buckets", "coarse", "--players", "2", "--iters", "60000", "--mem-gb", "0.4", "--eval-deals", "4000", "--seed", "3"].iter().map(|s| s.to_string()).collect();
        exploit::cli::run_json(&exploit::cli::parse_args(&a), compact)
    };
    let s = run(false, &ck);
    let c = run(true, &out);
    let (ms, mc) = (s["exploitation"]["avg_mbb"]["mbb"].as_f64().unwrap(), c["exploitation"]["avg_mbb"]["mbb"].as_f64().unwrap());
    let (cs, cc) = (s["exploitation"]["avg_mbb"]["ci95"].as_f64().unwrap(), c["exploitation"]["avg_mbb"]["ci95"].as_f64().unwrap());
    eprintln!("exploit store {ms:.0} +- {cs:.0}, compact {mc:.0} +- {cc:.0}");
    assert!(ms > 0.0 && mc > 0.0, "an unconverged blueprint is exploitable");
    // Hogwild learners and the u8 quantisation differ run to run; they must agree roughly.
    assert!((ms - mc).abs() < 0.35 * ms.max(mc) + 2.0 * (cs + cc), "{ms} vs {mc}");
    let _ = std::fs::remove_file(ck);
    let _ = std::fs::remove_file(out);
}
