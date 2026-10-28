//! CompactPolicy must play like BlueprintPolicy over the exported store, within u8
//! quantisation (every action probability within 1/255).

use bench::bp::{train_toy, BlueprintPolicy, CoarseBucketer};
use bench::compact::CompactPolicy;
use bench::policy::{Event, PolicyFactory, View};
use blueprint::compact::{compact_checkpoint, Compact, CompactOptions};
use blueprint::{io, snapshot};
use cards::Rng;
use nlhe::abstraction::BetSizing;
use nlhe::{Deal, GameConfig, State, Step};
use std::path::PathBuf;
use std::sync::Arc;

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("bench_compact_{}_{}", std::process::id(), name))
}

/// Play `hands` random hands where both policies track the same history; compare their
/// distributions at every decision. Returns (decisions, max |p_a - p_b|, action-set mismatches).
fn compare(a: &dyn PolicyFactory, b: &dyn PolicyFactory, players: usize, hands: u64) -> (u64, f64, u64) {
    let cfg = GameConfig::new(players);
    let (mut n, mut worst, mut mism) = (0u64, 0f64, 0u64);
    for h in 0..hands {
        let mut rng = Rng::new(0xC0FFEE ^ h);
        let deal = Deal::random(&mut rng);
        let mut pa: Vec<_> = (0..players).map(|s| a.new_player(s, &cfg, h)).collect();
        let mut pb: Vec<_> = (0..players).map(|s| b.new_player(s, &cfg, h)).collect();
        let mut st = State::new(&cfg);
        let mut len = 0usize;
        let mut hist: Vec<Event> = Vec::new();
        while !st.is_terminal() {
            let seat = st.to_act().unwrap();
            let view = View { cfg: &cfg, state: &st, board: &deal.board[..len], history: &hist, seat };
            let hole = deal.holes[seat];
            let da = pa[seat].dist(&view, hole);
            let db = pb[seat].dist(&view, hole);
            if da.len() != db.len() {
                mism += 1;
            }
            for i in 0..da.len() {
                worst = worst.max((da.prob(i) - db.prob_of(da.action(i))).abs());
            }
            n += 1;
            let action = da.action(da.sample(rng.f64()));
            let pre = st;
            let street = st.street();
            let step = st.apply(action);
            hist.push(Event { seat: seat as u8, street, action });
            for p in pa.iter_mut().chain(pb.iter_mut()) {
                p.observe(&pre, seat, action);
            }
            let mut reveal = |to: usize, len: &mut usize| {
                *len = to;
                for p in pa.iter_mut().chain(pb.iter_mut()) {
                    p.observe_board(&deal.board[..to]);
                }
            };
            match step {
                Step::Continue => {}
                Step::NewStreet(s) => reveal(s.board_len(), &mut len),
                Step::Terminal => {
                    if st.showdown() {
                        while len < 5 {
                            let to = if len == 0 { 3 } else { len + 1 };
                            reveal(to, &mut len);
                        }
                    }
                }
            }
        }
    }
    (n, worst, mism)
}

fn check(players: usize) {
    let bp = train_toy(players, 60_000, 3);
    let store = bp.store();
    let ck = tmp(&format!("ck{players}.bin"));
    let sums = tmp(&format!("sums{players}.bin"));
    let _ = std::fs::remove_file(&sums);
    // Two snapshots with a perturbation in between, so snapshot averaging differs from the
    // current strategy.
    snapshot::accumulate(store, &sums).unwrap();
    let mut r = Rng::new(17);
    for (_, slot) in store.entries() {
        let a = r.below(slot.n_actions as u32) as usize;
        store.add_regret(slot, a, r.below(4000) as f32 - 1000.0);
    }
    snapshot::accumulate(store, &sums).unwrap();
    io::save(store, &ck).unwrap();

    for use_sums in [false, true] {
        // Reference: the policy over the checkpoint store (exported if snapshots are used).
        let reference = io::load(&ck, 0, 0).unwrap();
        if use_sums {
            snapshot::export_in_place(&reference, Some(&sums)).unwrap();
        }
        let ref_pol = BlueprintPolicy::new("ref", Arc::new(reference), Arc::new(CoarseBucketer), BetSizing::tiny());
        let out = tmp(&format!("c{players}_{use_sums}.plcp"));
        let opt = CompactOptions { threads: 2, min_visits: 0, chunk: 4096, group_slots: 1 << 20, part_bytes: 1 << 20, tmp_dir: None };
        let stats = compact_checkpoint(&ck, use_sums.then_some(sums.as_path()), &out, &opt, &|_| {}).unwrap();
        let cp = CompactPolicy::new("compact", Arc::new(Compact::open(&out).unwrap()), Arc::new(CoarseBucketer), BetSizing::tiny());
        let (n, worst, mism) = compare(&ref_pol, &cp, players, 400);
        eprintln!("{players}p sums={use_sums}: {n} decisions, max dp {worst:.5}, mismatches {mism}, miss ref {:.4} compact {:.4}, {} infosets {} B", ref_pol.miss_rate(), cp.miss_rate(), stats.infosets_out, stats.bytes_out);
        assert!(n > 1000);
        assert_eq!(mism, 0);
        assert!(worst <= 1.0 / 255.0 + 1e-3, "max prob difference {worst}");
        assert!((ref_pol.miss_rate() - cp.miss_rate()).abs() < 1e-9, "same fallback behaviour");
        let _ = std::fs::remove_file(&out);
    }
    let _ = std::fs::remove_file(&ck);
    let _ = std::fs::remove_file(&sums);
}

#[test]
fn compact_policy_matches_blueprint_policy_heads_up() {
    check(2);
}

#[test]
fn compact_policy_matches_blueprint_policy_six_max() {
    check(6);
}
