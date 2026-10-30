//! Parallel external-sampling MCCFR with linear discounting and negative-regret pruning —
//! supplementary Algorithm 1 (MCCFR-P) on the abstract six-player NLHE game.
//!
//! Each iteration runs one traversal per player. A traversal samples a full deal up front
//! (external sampling samples chance once per traversal, so pre-dealing is equivalent),
//! enumerates the traverser's abstract actions and samples every opponent action from the
//! current regret-matching strategy. Threads share one `Store` and update it hogwild, as the
//! paper's 64-core run did.
//!
//! The paper schedules in wall-clock minutes; we schedule in iterations (`Schedule`), with
//! `Schedule::paper(iters_per_minute)` mapping the paper's minutes onto our measured speed.

use crate::store::{Slot, Store};
use cards::{Card, Rng};
use nlhe::abstraction::{abstract_actions, ActionList, BetSizing, HistKey, MAX_ACTIONS};
#[cfg(test)]
use nlhe::abstraction::History;
use nlhe::{Deal, GameConfig, State, Step, Street, MAX_PLAYERS};
use rayon::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

/// Card abstraction used by the trainer: bucket of a player's private cards + visible board.
pub trait Bucketer: Sync {
    /// `board` holds exactly the cards visible on `street` (0, 3, 4 or 5).
    fn bucket(&self, street: Street, hole: [Card; 2], board: &[Card]) -> u32;
    /// Number of buckets on `street` (bucket ids are `< n_buckets`).
    fn n_buckets(&self, street: Street) -> u32;
}

/// Iteration-based version of the paper's minute-based schedule.
#[derive(Clone, Debug)]
pub struct Schedule {
    /// Discount regrets and φ every `discount_interval` iterations while `t < lcfr_until`.
    pub discount_interval: u64,
    pub lcfr_until: u64,
    /// Start negative-regret pruning after this many iterations.
    pub prune_after: u64,
    /// Fraction of post-`prune_after` iterations that prune (paper: 0.95).
    pub prune_prob: f32,
    /// Regret threshold below which actions are skipped (paper: −300,000,000).
    pub prune_threshold: i32,
    /// Preflop average-strategy update every `strategy_interval` iterations (paper: 10,000).
    pub strategy_interval: u64,
    /// Only accumulate φ after this many iterations (paper: after 800 minutes).
    pub avg_after: u64,
}

impl Schedule {
    /// Paper schedule with minutes converted to iterations at `ipm` iterations per minute.
    pub fn paper(ipm: u64) -> Schedule {
        Schedule {
            discount_interval: 10 * ipm,
            lcfr_until: 400 * ipm,
            prune_after: 200 * ipm,
            prune_prob: 0.95,
            prune_threshold: -300_000_000,
            strategy_interval: 10_000,
            avg_after: 800 * ipm,
        }
    }
}

pub struct Trainer<'a, B: Bucketer> {
    pub game: GameConfig,
    pub sizing: BetSizing,
    pub sched: Schedule,
    pub bucketer: &'a B,
    pub store: std::sync::Arc<Store>,
    pub iterations: AtomicU64,
    pub nodes: AtomicU64,
    pub seed: u64,
}

thread_local! {
    /// Nodes visited by this thread since the last flush. A shared atomic bumped per node by
    /// 128 threads made one contended cache line the dominant cost of training (measured).
    static NODES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Per-traversal sampled chance outcome.
struct Ctx {
    deal: Deal,
    /// `buckets[seat][street]`.
    buckets: [[u32; 4]; MAX_PLAYERS],
}

impl<'a, B: Bucketer> Trainer<'a, B> {
    pub fn new(game: GameConfig, sizing: BetSizing, sched: Schedule, bucketer: &'a B, capacity: usize, seed: u64) -> Self {
        Trainer { game, sizing, sched, bucketer, store: std::sync::Arc::new(Store::new(capacity)), iterations: AtomicU64::new(0), nodes: AtomicU64::new(0), seed }
    }

    fn sample_ctx(&self, rng: &mut Rng) -> Ctx {
        let deal = Deal::random(rng);
        let mut buckets = [[0u32; 4]; MAX_PLAYERS];
        for (p, row) in buckets.iter_mut().enumerate().take(self.game.players) {
            for s in Street::ALL {
                row[s.idx()] = self.bucketer.bucket(s, deal.holes[p], &deal.board[..s.board_len()]);
            }
        }
        Ctx { deal, buckets }
    }

    /// The acting player's infoset, or `None` if the store is full and it was never visited.
    #[inline]
    fn slot(&self, st: &State, hist: &HistKey, ctx: &Ctx, n: usize) -> Option<Slot> {
        let actor = st.to_act().unwrap();
        let street = st.street().idx();
        let nb = self.bucketer.n_buckets(st.street()) as usize;
        self.store.try_infoset(hist.key(), ctx.buckets[actor][street], n, nb, street == 0)
    }

    /// TRAVERSE-MCCFR / TRAVERSE-MCCFR-P. Returns the traverser's utility in chips.
    fn traverse(&self, st: State, hist: HistKey, ctx: &Ctx, p: usize, prune: bool, rng: &mut Rng) -> f32 {
        if st.is_terminal() {
            return st.payoffs(&ctx.deal)[p] as f32;
        }
        if st.is_folded(p) {
            // Pi no longer in the hand: remaining actions are irrelevant to its payoff.
            return -st.contributed(p) as f32;
        }
        NODES.with(|c| c.set(c.get() + 1));
        let mut list = ActionList::new();
        abstract_actions(&st, &self.sizing, &mut list);
        let n = list.len();
        let mut sigma = [0f32; MAX_ACTIONS];
        let Some(slot) = self.slot(&st, &hist, ctx, n) else {
            // Store full and infoset never allocated: play uniformly, learn nothing here.
            let a = rng.below(n as u32) as usize;
            let mut s2 = st;
            s2.apply(list.get(a));
            let h2 = hist.pushed(a);
            return self.traverse(s2, h2, ctx, p, prune, rng);
        };
        self.store.strategy(slot, &mut sigma[..n]);

        if st.to_act() == Some(p) {
            let regrets = self.store.regrets(slot);
            let river = st.street() == Street::River;
            let mut va = [0f32; MAX_ACTIONS];
            let mut explored = [false; MAX_ACTIONS];
            let mut v = 0f32;
            for a in 0..n {
                let mut s2 = st;
                let step = s2.apply(list.get(a));
                if prune && !river && step != Step::Terminal && !s2.is_folded(p) && regrets[a].load(Relaxed) <= self.sched.prune_threshold {
                    continue;
                }
                let h2 = hist.pushed(a);
                va[a] = self.traverse(s2, h2, ctx, p, prune, rng);
                explored[a] = true;
                v += sigma[a] * va[a];
            }
            for a in 0..n {
                if explored[a] {
                    self.store.add_regret(slot, a, va[a] - v);
                }
            }
            v
        } else {
            let a = rng.sample_weighted(&sigma[..n]);
            let mut s2 = st;
            s2.apply(list.get(a));
            let h2 = hist.pushed(a);
            self.traverse(s2, h2, ctx, p, prune, rng)
        }
    }

    /// UPDATE-STRATEGY: accumulate the first-round average strategy φ for player `p`.
    /// Samples `p`'s own actions and enumerates everyone else's, stopping at the end of preflop.
    ///
    /// Two cost controls (measured: the literal version cut throughput 35–62% with large
    /// preflop menus): (1) opponent actions with probability exactly 0 under the current
    /// strategy are skipped — histories behind them are unreachable now and contribute no
    /// weight; (2) infosets are only *read* here, never allocated: an infoset the traversals
    /// never created has no regrets to average.
    fn update_strategy(&self, st: State, hist: HistKey, ctx: &Ctx, p: usize, rng: &mut Rng) {
        if st.is_terminal() || st.is_folded(p) || st.street() != Street::Preflop {
            return;
        }
        let mut list = ActionList::new();
        abstract_actions(&st, &self.sizing, &mut list);
        let n = list.len();
        let actor = st.to_act().unwrap();
        let Some(slot) = self.store.lookup(hist.key(), ctx.buckets[actor][0]) else { return };
        if slot.n_actions as usize != n || !slot.with_avg {
            return; // key collision with an infoset of another shape (see Store::try_infoset)
        }
        let mut sigma = [0f32; MAX_ACTIONS];
        self.store.strategy(slot, &mut sigma[..n]);
        if actor == p {
            let a = rng.sample_weighted(&sigma[..n]);
            self.store.bump_avg(slot, a);
            let mut s2 = st;
            s2.apply(list.get(a));
            self.update_strategy(s2, hist.pushed(a), ctx, p, rng);
        } else {
            for a in 0..n {
                if sigma[a] == 0.0 {
                    continue;
                }
                let mut s2 = st;
                s2.apply(list.get(a));
                self.update_strategy(s2, hist.pushed(a), ctx, p, rng);
            }
        }
    }

    /// One MCCFR iteration `t`: one traversal per player.
    pub fn iteration(&self, t: u64, rng: &mut Rng) {
        let root = State::new(&self.game);
        for p in 0..self.game.players {
            if t % self.sched.strategy_interval == 0 && t >= self.sched.avg_after {
                let ctx = self.sample_ctx(rng);
                self.update_strategy(root, HistKey::new(), &ctx, p, rng);
            }
            let prune = t > self.sched.prune_after && rng.f32() < self.sched.prune_prob;
            let ctx = self.sample_ctx(rng);
            self.traverse(root, HistKey::new(), &ctx, p, prune, rng);
        }
        let n = NODES.with(|c| c.replace(0));
        // Flush rarely-contended totals once per iteration.
        self.nodes.fetch_add(n, Relaxed);
        self.iterations.fetch_add(1, Relaxed);
    }

    /// Run iterations `[from, to)` in parallel, applying the LCFR discount at interval
    /// boundaries and calling `on_progress` between batches.
    pub fn run(&self, from: u64, to: u64, mut on_progress: impl FnMut(&Self, u64, f64)) {
        let start = Instant::now();
        let mut t = from;
        let batch = 4096u64;
        while t < to {
            // Never let a batch straddle a discount boundary.
            let mut end = (t + batch).min(to);
            let di = self.sched.discount_interval;
            if t < self.sched.lcfr_until {
                end = end.min((t / di + 1) * di);
            }
            let seed = self.seed;
            // One independent RNG stream per iteration (rayon may split work arbitrarily, so a
            // per-thread seed would replay identical deals across splits).
            (t..end).into_par_iter().for_each(|it| {
                let mut rng = Rng::new(seed ^ it.wrapping_mul(0xD1B5_4A32_D192_ED03));
                self.iteration(it, &mut rng)
            });
            t = end;
            if t < self.sched.lcfr_until && t % di == 0 {
                let k = (t / di) as f64;
                self.store.discount(k / (k + 1.0));
            }
            on_progress(self, t, start.elapsed().as_secs_f64());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Coarse bucketer for tests: preflop lossless, postflop by made-hand category.
    struct Coarse;
    impl Bucketer for Coarse {
        fn bucket(&self, street: Street, hole: [Card; 2], board: &[Card]) -> u32 {
            if street == Street::Preflop {
                return cards::preflop_class(hole[0], hole[1]) as u32;
            }
            let m = cards::mask_of(board) | cards::bit(hole[0]) | cards::bit(hole[1]);
            cards::category(cards::evaluate(m)) as u32
        }
        fn n_buckets(&self, street: Street) -> u32 {
            if street == Street::Preflop { 169 } else { 9 }
        }
    }

    #[test]
    fn heads_up_tiny_trains_and_learns_to_fold_trash() {
        let game = GameConfig::new(2);
        let sched = Schedule {
            discount_interval: 1000,
            lcfr_until: 20_000,
            prune_after: 10_000,
            prune_prob: 0.95,
            prune_threshold: -3_000_000,
            strategy_interval: 10,
            avg_after: 0,
        };
        let tr = Trainer::new(game, BetSizing::tiny(), sched, &Coarse, 1 << 24, 1);
        tr.run(0, 60_000, |_, _, _| {});
        assert!(tr.store.sequences() > 100);
        // Facing an all-in shove preflop as the BB, 72o should mostly fold and AA always call.
        let mut st = State::new(&game);
        let mut list = ActionList::new();
        abstract_actions(&st, &tr.sizing, &mut list);
        let shove = list.len() - 1;
        let mut h = History::new();
        st.apply(list.get(shove));
        h.push(shove);
        let probs = |class: usize| {
            let s = tr.store.lookup(h.key(), class as u32).expect("infoset visited");
            let mut p = [0f32; 2];
            tr.store.strategy(s, &mut p);
            p
        };
        let aa = probs(12);
        let trash = probs(cards::preflop_class(cards::parse_card("7c").unwrap(), cards::parse_card("2d").unwrap()));
        assert!(aa[1] > 0.95, "AA call prob {aa:?}");
        assert!(trash[0] > 0.8, "72o fold prob {trash:?}");
    }
}

