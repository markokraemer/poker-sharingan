//! Monte-Carlo Linear CFR over a depth-limited imperfect-information subgame
//! (supplementary "Further details of the real-time search algorithm", "Abstraction and
//! off-tree actions in subgames", "Leaf node values").
//!
//! One iteration `t` runs an external-sampling traversal per player that can still act:
//!
//! * the deal is sampled from the joint belief at the subgame root (rejection sampling gives
//!   exact card removal between players), the unseen board cards uniformly;
//! * the traverser enumerates its actions, everyone else samples from the current
//!   regret-matching strategy; regret and average-strategy updates are weighted by `t`
//!   (Linear CFR), with hogwild atomics (`Table`);
//! * the current betting round is solved with lossless (exact hole card) infosets, later
//!   rounds inside the subgame with the blueprint's bucketing by default, or with the paper's
//!   per-flop 500-bucket search abstraction when `SearchParams::search_buckets` is set;
//! * leaves (depth limit, see `tree::Limit`) are an extra decision: every player still able to
//!   act chooses one of the 4 continuation strategies, infoset = (public leaf, own bucket); the
//!   value of a joint choice is a rollout of the rest of the hand on the sampled deal with each
//!   player following its chosen continuation strategy (blueprint biased x5 toward fold / call /
//!   raise), each compressed to one deterministic action per abstract infoset.
//!
//! Randomness is seeded per iteration (`mix(seed, t)`), never per thread. With
//! `parallel = false` the whole solve is bit-reproducible.

use crate::beliefs::{hole_mask, holes, Beliefs, N};
use crate::bp::{cont_pick, real_passive, Blueprint, K_BLUEPRINT, NUM_CONT};
use crate::table::{Slot, Table};
use crate::tree::{limit_for, search_actions, step_path, Extras, Limit};
use crate::vector::{solve_vec, VecGame, VecParams, VecSolution};
use abstraction::search_buckets::{FlopView, SearchBuckets};
use bench::mix;
use cards::{bit, hole_index, Card, Rng};
use nlhe::abstraction::{ActionList, BetSizing, Shadow, MAX_ACTIONS};
use nlhe::{Action, Deal, State, Step, Street, MAX_PLAYERS};
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeafMode {
    /// Paper: each player picks one of 4 continuation strategies.
    Cont4,
    /// Ablation: every player plays the plain blueprint after the leaf.
    Blueprint,
}

#[derive(Clone, Debug)]
pub struct SearchParams {
    /// MCCFR iterations per search (each iteration = one traversal per acting player). Default 200k
    /// (measured: 6-max vs 5 x B, 20k it -42, 100k +58, 200k +87, 500k +58 mbb/game; see RESULTS.md).
    pub iters: u64,
    /// Optional wall-clock cap, checked between batches.
    pub max_millis: Option<u64>,
    /// Search action abstraction for the root round (1-6 raise sizes per node in the paper).
    pub sizing: BetSizing,
    /// Coarser abstraction for rounds after the root round inside the subgame (keeps solving
    /// to the end of the game affordable on one machine).
    pub later_sizing: BetSizing,
    /// Negative-regret pruning (Algorithm 1 MCCFR-P, applied to the subgame): after the first
    /// fifth of the iterations, in this fraction of them the traverser skips actions whose
    /// regret is below `-t^2/2 * prune_frac * stack` (a consistent per-iteration gap of
    /// `prune_frac * stack` chips). 0 disables.
    pub prune_prob: f32,
    pub prune_frac: f32,
    /// Warm start (deviation, compensates for far fewer iterations than the paper): a new
    /// infoset's regrets are initialised to `warm_start * t * pot * sigma_blueprint`, i.e. the
    /// blueprint's (translated) action probabilities, worth about `warm_start` iterations of
    /// evidence. 0 = start uniform.
    pub warm_start: f32,
    /// Infoset table capacity per search.
    pub max_infosets: usize,
    /// Fraction of our own traversals that fix our actual hole cards (importance of the infosets
    /// we actually play; unbiased per infoset, see crate docs).
    pub focus_own: f32,
    pub leaf: LeafMode,
    /// Multiplier of the biased continuation strategies (paper: 5).
    pub cont_bias: f32,
    /// Compress each continuation strategy to one sampled action per abstract infoset (paper).
    pub compress: bool,
    pub parallel: bool,
    pub seed: u64,
    /// Play the final iterate (paper) instead of the weighted average.
    pub play_final: bool,
    /// Search the first betting round when an opponent raise is far off-tree (paper).
    pub preflop_search: bool,
    /// "More than $100 off": chips; `None` = one big blind of the game.
    pub preflop_trigger_chips: Option<i32>,
    /// Preflop search only with at most this many players remaining (paper: 4).
    pub preflop_max_players: usize,
    /// Search flop/turn/river (paper: always). `false` = pure blueprint (ablation).
    pub postflop_search: bool,
    /// Paper's search abstraction: per-flop potential-aware 500-bucket tables for the turn and
    /// river inside the subgame (keys of later-round search infosets and of depth-limit leaves).
    /// `None` (default) = reuse the blueprint's bucketing. Blueprint lookups (warm-start prior,
    /// continuation-strategy rollouts, belief updates) always use the blueprint's own buckets.
    pub search_buckets: Option<Arc<SearchBuckets>>,
    /// Heads-up subgames from `vec_min_street` on are solved with vector-form Linear CFR
    /// (`vector.rs`: exact hole pairs and board cards, public chance sampling) instead of MC LCFR.
    pub vector: bool,
    pub vec_min_street: Street,
    /// Vector passes by root street `[flop, turn, river]`.
    pub vec_iters: [u64; 3],
    /// CFR+ style regret flooring in the vector solver (default; `false` = plain Linear CFR, worse here).
    pub vec_plus: bool,
    /// Pool the infosets of rounds after the root round by the blueprint's card buckets (shared
    /// across board runouts; far fewer passes needed, at the price of the abstraction).
    pub vec_pooled: bool,
    /// Search the flop (ablation: `false` = blueprint on the flop, search from the turn on).
    pub search_flop: bool,
    /// Search the turn (ablation).
    pub search_turn: bool,
    /// Search the river (ablation).
    pub search_river: bool,
    /// Update OPPONENT beliefs with the blueprint's likelihood instead of the search's average strategy
    /// (default: opponents are modelled as the blueprint; a measured +100 mbb/game at 20k iterations).
    pub blueprint_beliefs: bool,
    /// With `blueprint_beliefs`: still update OUR OWN seat's belief with the search average.
    pub own_search_beliefs: bool,
    /// With `blueprint_beliefs`: opponent actions that were off the search tree (added as extras) are
    /// still scored with the search's average strategy (the blueprint has no likelihood for them).
    pub offtree_search_beliefs: bool,
    /// Tremble ε on OPPONENT action likelihoods in the Bayes belief update: lik' = (1-ε)·lik + ε/n.
    /// Keeps hands an off-model opponent (e.g. a rule bot) really holds from being zeroed out when
    /// the model (blueprint/search) says "never". 0 = plain Bayes.
    pub belief_tremble: f32,
    /// Root-round infosets of the OTHER seats are keyed by the blueprint card bucket instead of the exact
    /// hole pair (we keep lossless infosets for our own seat). Opponents then share samples across the
    /// ~1326/200 hands of a bucket, like the blueprint they are modelled on.
    pub opp_pool: bool,
    /// Search a street only when at most this many players are still live at its start `[flop, turn, river]`
    /// (diagnostic / robustness knob; 6 = always).
    pub max_players: [usize; 3],
    /// Minimum live players for the flop search (diagnostic: isolate multiway vs HU flops; 2 = always).
    pub min_flop_players: usize,
    /// Dedicated thread pool for `parallel` solves (default: rayon's global pool).
    pub pool: Option<Arc<rayon::ThreadPool>>,
}

impl Default for SearchParams {
    fn default() -> Self {
        SearchParams {
            iters: 200_000,
            max_millis: None,
            sizing: crate::tree::default_sizing(),
            later_sizing: crate::tree::later_sizing(),
            prune_prob: 0.95,
            prune_frac: 0.01,
            warm_start: 8.0,
            max_infosets: 1 << 20,
            focus_own: 0.5,
            leaf: LeafMode::Cont4,
            cont_bias: 5.0,
            compress: true,
            parallel: true,
            seed: 1,
            play_final: false,
            preflop_search: true,
            preflop_trigger_chips: None,
            preflop_max_players: 4,
            postflop_search: true,
            search_buckets: None,
            vector: false,
            vec_min_street: Street::Turn,
            vec_iters: [300, 300, 150],
            vec_plus: true,
            vec_pooled: false,
            search_flop: true,
            search_turn: true,
            search_river: true,
            blueprint_beliefs: true,
            own_search_beliefs: true,
            offtree_search_beliefs: true,
            belief_tremble: 0.0,
            opp_pool: false,
            max_players: [6, 6, 6],
            min_flop_players: 2,
            pool: None,
        }
    }
}

/// Everything that defines one subgame solve.
pub struct Subgame<'a> {
    pub bp: &'a Blueprint,
    /// State at the start of the betting round (the subgame root).
    pub root: State,
    /// Board cards visible at the root.
    pub board: &'a [Card],
    /// Beliefs at the root (card removal vs `board` already applied).
    pub beliefs: &'a Beliefs,
    /// Shadow game at `root` (blueprint history of everything before the root, with off-menu
    /// raises replaced by their translations); see [`Blueprint::shadow`].
    pub bp_hist: Shadow,
    pub extras: &'a Extras,
    /// Our seat, our actual hole cards, and the actions we already took this round.
    pub me: usize,
    pub my_hole: Option<[Card; 2]>,
    pub frozen: &'a HashMap<u64, Action>,
    pub params: &'a SearchParams,
    pub seed: u64,
}

pub struct Solution {
    table: Table,
    vec: Option<VecSolution>,
    pub extras: Extras,
    pub sizing: BetSizing,
    pub root_street: Street,
    pub limit: Limit,
    pub iters: u64,
    pub millis: f64,
    pub nodes: u64,
    /// Depth-limit leaves evaluated (0 when solving to the end of the game).
    pub leaves: u64,
    /// Most raises seen so far this street at any root-round decision node.
    pub max_raises: u32,
    pub opp_pool: bool,
    pub me: usize,
    /// Per-flop search-bucket lookups performed (0 unless `SearchParams::search_buckets`).
    pub search_bucket_lookups: u64,
    /// Mean sampled value (chips) of each seat's own traversals at the subgame root under the
    /// strategies being learned, and the sample counts (MC solver only; zeros for vector).
    pub root_value: [(f64, u64); MAX_PLAYERS],
}

const TAG_LATER: u64 = 1 << 32;
const TAG_LEAF: u64 = 2 << 32;
const TAG_OPP: u64 = 3 << 32;
const VAL_STRIPES: usize = 256;

impl Solution {
    /// Strategy of the root-round infoset `(path, hole)` over `n` actions: the final iterate
    /// or the weighted average. `false` if the infoset was never reached.
    pub fn probs(&self, path: u64, hole: usize, n: usize, final_iterate: bool, out: &mut [f32]) -> bool {
        if let Some(v) = &self.vec {
            return v.probs(path, hole, n, final_iterate, out);
        }
        let Some(slot) = self.table.find(Table::key(path, hole as u64)).filter(|s| s.n == n) else {
            return false;
        };
        if final_iterate {
            self.table.strategy(slot, &mut out[..n]);
            true
        } else {
            self.table.average(slot, &mut out[..n])
        }
    }
    /// Strategy of another seat's pooled root-round infoset `(path, blueprint bucket)` (`opp_pool` solves).
    pub fn probs_opp(&self, path: u64, bucket: u32, n: usize, final_iterate: bool, out: &mut [f32]) -> bool {
        let Some(slot) = self.table.find(Table::key(path, TAG_OPP | bucket as u64)).filter(|s| s.n == n) else {
            return false;
        };
        if final_iterate {
            self.table.strategy(slot, &mut out[..n]);
            true
        } else {
            self.table.average(slot, &mut out[..n])
        }
    }
    pub fn infosets(&self) -> usize {
        self.vec.as_ref().map_or_else(|| self.table.infosets(), |v| v.nodes())
    }
    /// The vector-form solution, if the subgame was solved that way.
    /// Peak node storage (bytes) of a vector solve, 0 for MC.
    pub fn peak_bytes(&self) -> usize {
        self.vec.as_ref().map_or(0, |v| v.peak_bytes)
    }
    pub fn vector(&self) -> Option<&VecSolution> {
        self.vec.as_ref()
    }
    /// Later-round strategy of the MC solver: `(path, street, bucket)` (board-agnostic keys).
    pub fn probs_later(&self, path: u64, street: Street, bucket: u32, n: usize, final_iterate: bool, out: &mut [f32]) -> bool {
        let key = Table::key(path, TAG_LATER | (street.idx() as u64) << 24 | bucket as u64);
        let Some(slot) = self.table.find(key).filter(|s| s.n == n) else { return false };
        if final_iterate {
            self.table.strategy(slot, &mut out[..n]);
            true
        } else {
            self.table.average(slot, &mut out[..n])
        }
    }
}

struct Cx {
    deal: Deal,
    hole: [u16; MAX_PLAYERS],
    /// Blueprint buckets (blueprint lookups: warm-start prior, rollouts).
    bucket: [[u32; 4]; MAX_PLAYERS],
    /// Buckets used as search infoset keys: per-flop search buckets when enabled, else `bucket`.
    sbucket: [[u32; 4]; MAX_PLAYERS],
    rng: Rng,
    t: f32,
    /// Regret threshold below which the traverser skips an action this iteration (or -inf).
    prune_at: f32,
    /// Per-traversal counters, flushed once (shared atomics per node made parallel solves ~10x slower).
    nodes: u64,
    leaves: u64,
    sb: u64,
}

struct Solver<'a> {
    sg: &'a Subgame<'a>,
    table: Table,
    limit: Limit,
    root_street: Street,
    n: usize,
    live: u8,
    cdf: Vec<Vec<f64>>,
    board_mask: u64,
    my_idx: Option<u16>,
    salt: u64,
    nodes: AtomicU64,
    leaves: AtomicU64,
    max_raises: AtomicU32,
    stack: f32,
    view: Option<FlopView>,
    sb_lookups: AtomicU64,
    /// Sum and count of the traverser's sampled root values per seat (diagnostics).
    val: Vec<std::sync::Mutex<[(f64, u64); MAX_PLAYERS]>>,
}

fn hash01(a: u64, b: u64, c: u64, d: u64) -> f64 {
    (mix(mix(mix(a, b), c), d) >> 11) as f64 / (1u64 << 53) as f64
}

/// Solve the subgame and return the strategies.
pub fn solve(sg: &Subgame) -> Solution {
    let start = Instant::now();
    let root = sg.root;
    let n = root.num_players();
    let p = sg.params;
    if p.vector && root.street() >= p.vec_min_street && root.street() != Street::Preflop {
        if let Some(seats) = VecGame::live_seats(&root) {
            return solve_vector(sg, seats, start);
        }
    }
    let live = (0..n).filter(|&i| !root.is_folded(i)).fold(0u8, |m, i| m | 1 << i);
    let solver = Solver {
        sg,
        table: Table::new(p.max_infosets, p.max_infosets * 8),
        limit: limit_for(&root),
        root_street: root.street(),
        n,
        live,
        cdf: (0..n).map(|i| sg.beliefs.cdf(i)).collect(),
        board_mask: cards::mask_of(sg.board),
        my_idx: sg.my_hole.map(|h| hole_index(h[0], h[1]) as u16),
        salt: mix(sg.seed, 0xC0FFEE),
        nodes: AtomicU64::new(0),
        leaves: AtomicU64::new(0),
        max_raises: AtomicU32::new(0),
        stack: (0..n).map(|i| root.stack(i) + root.contributed(i)).max().unwrap_or(0) as f32,
        view: match &p.search_buckets {
            Some(sb) if root.street() != Street::Preflop && sg.board.len() >= 3 => Some(sb.for_flop([sg.board[0], sg.board[1], sg.board[2]])),
            _ => None,
        },
        sb_lookups: AtomicU64::new(0),
        val: (0..VAL_STRIPES).map(|_| std::sync::Mutex::new([(0.0, 0); MAX_PLAYERS])).collect(),
    };
    let players: Vec<usize> = if root.is_terminal() {
        vec![]
    } else {
        (0..n).filter(|&i| live >> i & 1 != 0 && !root.is_all_in(i)).collect()
    };
    let batch = if p.parallel { 256u64 } else { 1 << 20 };
    let mut done = 0u64;
    let drive = |done: &mut u64| {
        let mut t0 = 1u64;
        while t0 <= p.iters && !players.is_empty() {
            let end = (t0 + batch - 1).min(p.iters);
            let run = |t: u64| solver.iteration(t, &players);
            if p.parallel {
                (t0..=end).into_par_iter().for_each(run);
            } else {
                (t0..=end).for_each(run);
            }
            *done = end;
            t0 = end + 1;
            if p.max_millis.is_some_and(|ms| start.elapsed().as_millis() as u64 >= ms) {
                break;
            }
        }
    };
    match (&p.pool, p.parallel) {
        // A dedicated pool is driven from a plain OS thread: blocking a rayon worker of another
        // pool in `install` would let it steal unrelated jobs (nested searches) meanwhile.
        (Some(pl), true) => std::thread::scope(|sc| {
            sc.spawn(|| pl.install(|| drive(&mut done))).join().unwrap();
        }),
        _ => drive(&mut done),
    }
    Solution {
        table: solver.table,
        vec: None,
        extras: sg.extras.clone(),
        sizing: p.sizing,
        root_street: solver.root_street,
        limit: solver.limit,
        iters: done,
        millis: start.elapsed().as_secs_f64() * 1e3,
        nodes: solver.nodes.load(Relaxed),
        leaves: solver.leaves.load(Relaxed),
        max_raises: solver.max_raises.load(Relaxed),
        opp_pool: p.opp_pool && solver.root_street != Street::Preflop,
        me: sg.me,
        search_bucket_lookups: solver.sb_lookups.load(Relaxed),
        root_value: {
            let mut tot = [(0.0f64, 0u64); MAX_PLAYERS];
            for m in &solver.val {
                let g = m.lock().unwrap();
                for i in 0..MAX_PLAYERS {
                    tot[i].0 += g[i].0;
                    tot[i].1 += g[i].1;
                }
            }
            let mut o = [(0.0, 0u64); MAX_PLAYERS];
            for i in 0..MAX_PLAYERS {
                o[i] = (if tot[i].1 > 0 { tot[i].0 / tot[i].1 as f64 } else { 0.0 }, tot[i].1);
            }
            o
        },
    }
}

fn solve_vector(sg: &Subgame, seats: [usize; 2], start: Instant) -> Solution {
    let p = sg.params;
    let root = sg.root;
    let bm = cards::mask_of(sg.board);
    let ranges = seats.map(|s| {
        sg.beliefs.w[s].iter().enumerate().map(|(i, &x)| if hole_mask(i) & bm == 0 { x as f64 } else { 0.0 }).collect::<Vec<f64>>()
    });
    let g = VecGame {
        root,
        board: sg.board,
        seats,
        ranges,
        sizing: &p.sizing,
        later: &p.later_sizing,
        extras: sg.extras,
        me: seats.iter().position(|&s| s == sg.me),
        my_hole: sg.my_hole.map(|h| hole_index(h[0], h[1])),
        frozen: sg.frozen,
        pooled: if p.vec_pooled { Some(sg.bp as &dyn crate::vector::Bucketing) } else { None },
    };
    let iters = p.vec_iters[(root.street().idx() - 1).min(2)];
    let vp = VecParams { iters, max_millis: p.max_millis, plus: p.vec_plus, stratify: true, seed: sg.seed, keep_all: false, max_bytes: 2 << 30 };
    let vs = solve_vec(&g, &vp);
    Solution {
        table: Table::new(1, 8),
        extras: sg.extras.clone(),
        sizing: p.sizing,
        root_street: root.street(),
        limit: Limit::EndOfGame,
        iters: vs.iters,
        millis: start.elapsed().as_secs_f64() * 1e3,
        nodes: 0,
        leaves: 0,
        max_raises: 0,
        opp_pool: false,
        me: sg.me,
        search_bucket_lookups: 0,
        root_value: [(0.0, 0); MAX_PLAYERS],
        vec: Some(vs),
    }
}

impl<'a> Solver<'a> {
    fn iteration(&self, t: u64, players: &[usize]) {
        let mut rng = Rng::new(mix(self.sg.seed, t));
        for &p in players {
            let force = p == self.sg.me && self.my_idx.is_some() && rng.f32() < self.sg.params.focus_own;
            if let Some(mut cx) = self.sample(&mut rng, force, t) {
                let prm = self.sg.params;
                if prm.prune_prob > 0.0 && t * 5 > prm.iters && rng.f32() < prm.prune_prob {
                    // Cumulative Linear-CFR regret of a consistently bad action grows like
                    // t^2/2 x its per-iteration gap.
                    let t = t as f32;
                    cx.prune_at = -0.5 * t * t * prm.prune_frac * self.stack;
                }
                let bph = self.sg.bp_hist;
                let v = self.walk(&mut cx, &self.sg.root, 0, &bph, p);
                self.nodes.fetch_add(cx.nodes, Relaxed);
                self.leaves.fetch_add(cx.leaves, Relaxed);
                if cx.sb > 0 {
                    self.sb_lookups.fetch_add(cx.sb, Relaxed);
                }
                // Diagnostic root values: per-thread stripes (a single mutex serialised all threads).
                {
                    let mut g = self.val[rayon::current_thread_index().unwrap_or(0) % VAL_STRIPES].lock().unwrap();
                    g[p].0 += v as f64;
                    g[p].1 += 1;
                }
            }
        }
    }

    /// Sample a deal: holes from the joint belief (rejection => exact card removal), then the
    /// unseen board uniformly from the remaining deck.
    fn sample(&self, rng: &mut Rng, force_me: bool, t: u64) -> Option<Cx> {
        let hl = holes();
        let mut idx = [0u16; MAX_PLAYERS];
        let mut ok = false;
        for _ in 0..64 {
            let mut dead = self.board_mask;
            let mut good = true;
            for s in 0..self.n {
                if self.live >> s & 1 == 0 {
                    continue;
                }
                let i = if force_me && s == self.sg.me {
                    self.my_idx.unwrap() as usize
                } else {
                    let c = &self.cdf[s];
                    let tot = *c.last().unwrap();
                    if tot <= 0.0 {
                        good = false;
                        break;
                    }
                    let x = rng.f64() * tot;
                    c.partition_point(|&v| v <= x).min(N - 1)
                };
                let m = hole_mask(i);
                if m & dead != 0 {
                    good = false;
                    break;
                }
                dead |= m;
                idx[s] = i as u16;
            }
            if good {
                ok = true;
                break;
            }
        }
        if !ok {
            return None;
        }
        let mut dead = self.board_mask;
        let mut deal = Deal { holes: [[0, 1]; MAX_PLAYERS], board: [0; 5] };
        for s in 0..self.n {
            if self.live >> s & 1 != 0 {
                deal.holes[s] = hl[idx[s] as usize];
                dead |= hole_mask(idx[s] as usize);
            }
        }
        for (i, c) in self.sg.board.iter().enumerate() {
            deal.board[i] = *c;
        }
        for i in self.sg.board.len()..5 {
            let c = rng.draw(dead);
            dead |= bit(c);
            deal.board[i] = c;
        }
        let mut bucket = [[0u32; 4]; MAX_PLAYERS];
        for s in 0..self.n {
            if self.live >> s & 1 == 0 {
                continue;
            }
            for st in Street::ALL {
                if st >= self.root_street {
                    bucket[s][st.idx()] = self.sg.bp.bucket(st, deal.holes[s], &deal.board[..st.board_len()]);
                }
            }
        }
        let mut sbucket = bucket;
        let mut sb_n = 0u64;
        if let Some(v) = &self.view {
            for s in 0..self.n {
                if self.live >> s & 1 == 0 {
                    continue;
                }
                for st in [Street::Turn, Street::River] {
                    if st >= self.root_street {
                        if let Some(b) = v.bucket(deal.holes[s], &deal.board[..st.board_len()]) {
                            sbucket[s][st.idx()] = b;
                            sb_n += 1;
                        }
                    }
                }
            }
        }
        Some(Cx { deal, hole: idx, bucket, sbucket, rng: Rng::new(mix(rng.next_u64(), t)), t: t as f32, prune_at: f32::NEG_INFINITY, nodes: 0, leaves: 0, sb: sb_n })
    }

    #[inline]
    fn key(&self, cx: &Cx, st: &State, path: u64, seat: usize) -> u64 {
        if st.street() == self.root_street {
            if self.sg.params.opp_pool && seat != self.sg.me && self.root_street != Street::Preflop {
                Table::key(path, TAG_OPP | cx.bucket[seat][st.street().idx()] as u64)
            } else {
                Table::key(path, cx.hole[seat] as u64)
            }
        } else {
            Table::key(path, TAG_LATER | (st.street().idx() as u64) << 24 | cx.sbucket[seat][st.street().idx()] as u64)
        }
    }

    fn walk(&self, cx: &mut Cx, st: &State, path: u64, bph: &Shadow, p: usize) -> f32 {
        cx.nodes += 1;
        if st.street() == self.root_street && st.raises_this_street() > self.max_raises.load(Relaxed) {
            self.max_raises.fetch_max(st.raises_this_street(), Relaxed);
        }
        let seat = st.to_act().unwrap();
        let mut list = ActionList::new();
        let ex: &[i32] = self.sg.extras.get(&path).map_or(&[], |v| v.as_slice());
        let sz = if st.street() == self.root_street { &self.sg.params.sizing } else { &self.sg.params.later_sizing };
        search_actions(st, sz, ex, &mut list);
        let n = list.len();
        let key = self.key(cx, st, path, seat);
        let slot = if self.sg.params.warm_start > 0.0 {
            let lam = self.sg.params.warm_start * cx.t * st.pot() as f32;
            let bucket = cx.bucket[seat][st.street().idx()];
            self.table.get_or_insert_with(key, n, |r| {
                if let Some(pr) = self.sg.bp.prior(bph, st, &list, bucket) {
                    for (x, &q) in r.iter_mut().zip(&pr[..n]) {
                        *x = lam * q;
                    }
                }
            })
        } else {
            self.table.get_or_insert(key, n)
        };
        let mut sigma = [0f32; MAX_ACTIONS];
        match slot {
            Some(s) => self.table.strategy(s, &mut sigma[..n]),
            None => sigma[..n].iter_mut().for_each(|x| *x = 1.0 / n as f32),
        }
        // Frozen: our own earlier action this round, for our actual hand only.
        let mut frozen = None;
        if seat == self.sg.me && Some(cx.hole[seat]) == self.my_idx {
            if let Some(fa) = self.sg.frozen.get(&path) {
                frozen = list.as_slice().iter().position(|a| a == fa);
            }
        }
        if let Some(f) = frozen {
            sigma[..n].iter_mut().for_each(|x| *x = 0.0);
            sigma[f] = 1.0;
            if let Some(s) = slot {
                // Readers (dist, belief updates) see the pinned pure strategy.
                self.table.set_pure(s, f);
                self.table.add_avg(s, f, 1.0);
            }
        }
        if seat == p {
            let mut va = [0f32; MAX_ACTIONS];
            let mut v = 0f32;
            let mut explored = [true; MAX_ACTIONS];
            let mut mass = 0f32;
            for a in 0..n {
                if frozen.is_some_and(|f| f != a) {
                    explored[a] = false;
                    continue;
                }
                if cx.prune_at.is_finite() && !matches!(list.get(a), Action::Fold) {
                    if let Some(s) = slot {
                        if self.table.regret(s, a) < cx.prune_at && !self.ends_hand(st, list.get(a)) {
                            explored[a] = false;
                            continue;
                        }
                    }
                }
                va[a] = self.child(cx, st, path, bph, &list, a, p);
                v += sigma[a] * va[a];
                mass += sigma[a];
            }
            if mass < 1.0 {
                // Pruned actions left out of the expectation: renormalise over explored ones.
                if mass > 0.0 {
                    v /= mass;
                } else {
                    let k = (0..n).filter(|&a| explored[a]).count().max(1);
                    v = (0..n).filter(|&a| explored[a]).map(|a| va[a]).sum::<f32>() / k as f32;
                }
            }
            if frozen.is_none() {
                if let Some(s) = slot {
                    for a in 0..n {
                        if explored[a] {
                            self.table.add_regret(s, a, cx.t * (va[a] - v));
                        }
                    }
                }
            }
            v
        } else {
            let a = cx.rng.sample_weighted(&sigma[..n]);
            if frozen.is_none() {
                if let Some(s) = slot {
                    for (i, &x) in sigma[..n].iter().enumerate() {
                        self.table.add_avg(s, i, cx.t * x);
                    }
                }
            }
            self.child(cx, st, path, bph, &list, a, p)
        }
    }

    /// Does `a` end the hand immediately (never pruned, as in Algorithm 1)?
    fn ends_hand(&self, st: &State, a: Action) -> bool {
        let mut s2 = *st;
        s2.apply(a) == Step::Terminal
    }

    #[allow(clippy::too_many_arguments)]
    fn child(&self, cx: &mut Cx, st: &State, path: u64, bph: &Shadow, list: &ActionList, a: usize, p: usize) -> f32 {
        let act = list.get(a);
        let mut s2 = *st;
        let step = s2.apply(act);
        if s2.is_folded(p) {
            return -(s2.contributed(p) as f32);
        }
        let path2 = step_path(path, act);
        if step == Step::Terminal {
            return s2.payoffs(&cx.deal)[p] as f32;
        }
        let mut b2 = *bph;
        self.sg.bp.observe(&mut b2, st, act, st.to_act() == Some(self.sg.me));
        match step {
            Step::NewStreet(s) if self.limit != Limit::EndOfGame => {
                let known = Street::ALL[s.idx() - 1];
                self.leaf(cx, &s2, path2, &b2, known, p)
            }
            Step::Continue
                if self.limit == Limit::SecondRaise
                    && s2.street() == self.root_street
                    && s2.raises_this_street() >= 2 =>
            {
                self.leaf(cx, &s2, path2, &b2, s2.street(), p)
            }
            _ => self.walk(cx, &s2, path2, &b2, p),
        }
    }

    /// Depth-limit leaf: continuation-strategy choice (supplement, "Leaf node values").
    fn leaf(&self, cx: &mut Cx, st: &State, path: u64, bph: &Shadow, known: Street, p: usize) -> f32 {
        cx.leaves += 1;
        let mut ks = [K_BLUEPRINT; MAX_PLAYERS];
        if self.sg.params.leaf == LeafMode::Blueprint {
            return self.rollout(cx, st, bph, &ks, p);
        }
        let lpath = path.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x1EAF;
        let mut mine: Option<Slot> = None;
        let mut p_chooses = false;
        for i in 0..self.n {
            if st.is_folded(i) || st.is_all_in(i) {
                continue;
            }
            let key = Table::key(lpath, TAG_LEAF | cx.sbucket[i][known.idx()] as u64);
            let slot = self.table.get_or_insert(key, NUM_CONT);
            if i == p {
                p_chooses = true;
                mine = slot;
                continue;
            }
            let mut sg = [0.25f32; NUM_CONT];
            if let Some(s) = slot {
                self.table.strategy(s, &mut sg);
            }
            ks[i] = cx.rng.sample_weighted(&sg) as u8;
            if let Some(s) = slot {
                for (k, &x) in sg.iter().enumerate() {
                    self.table.add_avg(s, k, cx.t * x);
                }
            }
        }
        if !p_chooses {
            return self.rollout(cx, st, bph, &ks, p);
        }
        let mut sg = [0.25f32; NUM_CONT];
        if let Some(s) = mine {
            self.table.strategy(s, &mut sg);
        }
        let mut va = [0f32; NUM_CONT];
        let mut v = 0.0;
        for k in 0..NUM_CONT {
            ks[p] = k as u8;
            va[k] = self.rollout(cx, st, bph, &ks, p);
            v += sg[k] * va[k];
        }
        if let Some(s) = mine {
            for k in 0..NUM_CONT {
                self.table.add_regret(s, k, cx.t * (va[k] - v));
            }
        }
        v
    }

    /// Play the hand out on the sampled deal; seat `i` follows continuation strategy `ks[i]`
    /// of the blueprint, compressed to one action per abstract infoset.
    fn rollout(&self, cx: &mut Cx, st: &State, bph: &Shadow, ks: &[u8; MAX_PLAYERS], p: usize) -> f32 {
        let bp = self.sg.bp;
        let prm = self.sg.params;
        let mut s = *st;
        let mut h = *bph;
        let mut list = ActionList::new();
        let mut sig = [0f32; MAX_ACTIONS];
        while !s.is_terminal() {
            let seat = s.to_act().unwrap();
            let bucket = cx.bucket[seat][s.street().idx()];
            let a = if bp.node_list(&h, &s, &mut list) && bp.sigma(&h, &s, &list, bucket, &mut sig) {
                let n = list.len();
                let u = if prm.compress {
                    hash01(self.salt, ks[seat] as u64, h.key(), bucket as u64)
                } else {
                    cx.rng.f64()
                };
                let idx = cont_pick(ks[seat], prm.cont_bias, &list, &mut sig[..n], u);
                h.real_action(&s, &list, idx)
            } else {
                real_passive(&s)
            };
            bp.observe(&mut h, &s, a, true);
            s.apply(a);
        }
        s.payoffs(&cx.deal)[p] as f32
    }
}

/// Number of hole pairs with positive mass in `b` for `seat` (diagnostics).
pub fn support(b: &Beliefs, seat: usize) -> usize {
    b.w[seat].iter().filter(|&&x| x > 0.0).count()
}
