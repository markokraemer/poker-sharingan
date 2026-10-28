//! Helpers shared by tests and examples: small blueprints trained on the spot.

use crate::bp::Blueprint;
use bench::bp::CoarseBucketer;
use blueprint::store::Store;
use blueprint::trainer::{Bucketer, Schedule, Trainer};
use nlhe::abstraction::BetSizing;
use nlhe::GameConfig;
use std::path::PathBuf;
use std::sync::Arc;

/// A real subgame root reached by blueprint self-play, with the blueprint's Bayes beliefs.
pub struct Spot {
    pub root: nlhe::State,
    pub bph: nlhe::abstraction::Shadow,
    pub board: Vec<cards::Card>,
    pub beliefs: crate::beliefs::Beliefs,
}

/// Play one hand of blueprint self-play (no search) until the start of `street`'s betting round
/// and return that subgame root with each seat's belief (row `s` taken from another seat's view,
/// so the viewer's own-hand floor does not leak). `None` if the hand ended before.
pub fn sample_spot(bp: &Blueprint, cfg: &GameConfig, street: nlhe::Street, seed: u64) -> Option<Spot> {
    use crate::policy::SearchPolicy;
    use crate::solver::SearchParams;
    use bench::policy::{Player, View};
    use nlhe::{Deal, State, Step};
    let prm = SearchParams { postflop_search: false, preflop_search: false, ..Default::default() };
    let pol = SearchPolicy::new("spot", bp.clone(), prm);
    let mut rng = cards::Rng::new(seed);
    let deal = Deal::random(&mut rng);
    let n = cfg.players;
    let mut ps: Vec<_> = (0..n).map(|s| pol.player(s, cfg, bench::mix(seed, s as u64))).collect();
    let mut st = State::new(cfg);
    loop {
        let seat = st.to_act()?;
        let board = &deal.board[..st.street().board_len()];
        let v = View { cfg, state: &st, board, history: &[], seat };
        let d = ps[seat].dist(&v, deal.holes[seat]);
        let a = d.action(d.sample(rng.f64()));
        let pre = st;
        let step = st.apply(a);
        for p in ps.iter_mut() {
            p.observe(&pre, seat, a);
        }
        match step {
            Step::Continue => {}
            Step::Terminal => return None,
            Step::NewStreet(s) => {
                let b = &deal.board[..s.board_len()];
                for p in ps.iter_mut() {
                    p.observe_board(b);
                }
                if s == street {
                    let mut beliefs = ps[0].beliefs().clone();
                    for s in 0..n {
                        beliefs.w[s] = ps[(s + 1) % n].beliefs().w[s].clone();
                    }
                    let (root, bph, board) = ps[0].round_root();
                    return Some(Spot { root, bph, board: board.to_vec(), beliefs });
                }
            }
        }
    }
}

/// `data/buckets-200` (relative to the workspace), if present.
pub fn bucket_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/buckets-200")
}

/// The real 169/200/200/200 bucketer when the tables exist, else the coarse test bucketer.
pub fn best_bucketer() -> (Arc<dyn Bucketer + Send>, &'static str) {
    if let Ok(t) = abstraction::tables::BucketTables::load(&bucket_dir()) {
        (Arc::new(pluribus::TableBucketer(t)), "buckets-200")
    } else {
        (Arc::new(CoarseBucketer), "coarse")
    }
}

/// Toy schedule: LCFR discounts for the first third, pruning from the second sixth, preflop
/// average accumulated throughout.
pub fn toy_schedule(iters: u64) -> Schedule {
    Schedule {
        discount_interval: 1000,
        lcfr_until: iters / 3,
        prune_after: iters / 6,
        prune_prob: 0.95,
        prune_threshold: -3_000_000,
        strategy_interval: 10,
        avg_after: 0,
    }
}

/// Train a blueprint for `players` seats with `sizing` and `bucketer` for `iters` iterations.
pub fn train_blueprint<B: Bucketer + Send + Sync + 'static>(
    players: usize,
    sizing: BetSizing,
    bucketer: Arc<B>,
    iters: u64,
    seed: u64,
) -> Blueprint {
    let tr = Trainer::new(GameConfig::new(players), sizing, toy_schedule(iters), &*bucketer, 0, seed);
    let mut tr = tr;
    tr.store = Arc::new(Store::with_index(1 << 26, 1 << 23));
    tr.run(0, iters, |_, _, _| {});
    let store = std::mem::replace(&mut tr.store, Arc::new(Store::new(16)));
    drop(tr);
    Blueprint::new(store, bucketer, sizing)
}
