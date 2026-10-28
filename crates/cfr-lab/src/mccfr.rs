//! External-sampling MCCFR with negative-regret pruning: Pluribus supplementary,
//! Algorithm 1 ("MCCFR-P"), Brown & Sandholm, Science 2019. Lambda & Lanctot 2009 for ES.
//!
//! Per iteration `t`, for each player `P_i` in turn:
//!  1. if `t % strategy_interval == 0`: `UPDATE-STRATEGY` - walk the tree sampling chance
//!     and `P_i`'s own actions from the current strategy, enumerating opponents; the sampled
//!     action's counter `phi(I, a)` is incremented (only for rounds `< avg_rounds`;
//!     Pluribus: 1 = first betting round only).
//!  2. if `t > prune_after`: with prob. `1 - prune_prob` run plain `TRAVERSE-MCCFR`,
//!     otherwise `TRAVERSE-MCCFR-P`; before that, plain `TRAVERSE-MCCFR`.
//!  Traversal: `P_i` enumerates its actions and updates regret `R += v(a) - v` (i32,
//!  scaled by `utility_scale`, floored at `regret_floor`); opponents and chance are sampled.
//!  `-P` skips traverser actions with `R <= C` unless on the last betting round or the
//!  action leads directly to a terminal node.
//!  3. if `t < lcfr_until && t % discount_interval == 0`: multiply all `R` and `phi` by
//!     `d = (t/D) / (t/D + 1)` (linear weighting, Pluribus uses minutes; here iterations).
//!
//! Pluribus numbers -> small-game scale are in [`MccfrConfig::small_game`].

use crate::cfr::regret_match;
use crate::tree::{Kind, Profile, Tree};
use crate::{Solver, MAX_ACTIONS};
use cards::Rng;

#[derive(Clone, Debug)]
pub struct MccfrConfig {
    /// `Strategy Interval` (10,000 iterations in Pluribus).
    pub strategy_interval: u64,
    /// `Prune Threshold`: iterations after which pruning may start (200 min in Pluribus).
    pub prune_after: u64,
    /// Fraction of iterations that prune (0.95).
    pub prune_prob: f64,
    /// `C`, in scaled regret units (-300,000,000 in Pluribus).
    pub prune_threshold: i32,
    /// Regret floor in scaled units (-310,000,000 in Pluribus).
    pub regret_floor: i32,
    /// `LCFR Threshold` (400 min).
    pub lcfr_until: u64,
    /// `Discount Interval` (10 min).
    pub discount_interval: u64,
    /// Chips -> integer regret units (Pluribus' chips are already integers; ours are
    /// expectations over sampled continuations so need a fixed-point scale).
    pub utility_scale: f64,
    /// Average strategy tracked for rounds `< avg_rounds`.
    pub avg_rounds: usize,
    pub seed: u64,
}

impl MccfrConfig {
    /// Pluribus' schedule scaled for Leduc/Kuhn (~10^4..10^6 iterations instead of 10^9+):
    /// discount every 10 iterations until 40% of the run; prune after 20% of the run in
    /// 95% of iterations with C = -5 chips (Pluribus: -300M of a 10,000-chip stack at
    /// integer chips => ~ 3e4 stacks; here a stack is ~10 chips, and C is deliberately
    /// tight so the pruning path is really exercised); strategy snapshots every iteration
    /// (tiny trees make UPDATE-STRATEGY cheap); regret floor 10% below C. Regrets use
    /// 1/1000-chip fixed point.
    pub fn small_game(total_iters: u64, seed: u64) -> Self {
        let scale = 1000.0;
        MccfrConfig {
            strategy_interval: 1,
            prune_after: total_iters / 5,
            prune_prob: 0.95,
            prune_threshold: (-5.0 * scale) as i32,
            regret_floor: (-5.5 * scale) as i32,
            lcfr_until: total_iters * 2 / 5,
            discount_interval: 10,
            utility_scale: scale,
            avg_rounds: usize::MAX,
            seed,
        }
    }
}

pub struct Mccfr<'a> {
    tree: &'a Tree,
    pub cfg: MccfrConfig,
    regret: Vec<i32>,
    phi: Vec<f64>,
    t: u64,
    rng: Rng,
    /// Number of traverser actions skipped by negative-regret pruning so far.
    pub pruned: u64,
}

fn sample(rng: &mut Rng, w: &[f64]) -> usize {
    let mut x = rng.f64() * w.iter().sum::<f64>();
    for (i, &p) in w.iter().enumerate() {
        if x < p {
            return i;
        }
        x -= p;
    }
    w.len() - 1
}

impl<'a> Mccfr<'a> {
    pub fn new(tree: &'a Tree, cfg: MccfrConfig) -> Self {
        let rng = Rng::new(cfg.seed);
        Mccfr {
            tree,
            regret: vec![0; tree.total_actions],
            phi: vec![0.0; tree.total_actions],
            t: 0,
            rng,
            pruned: 0,
            cfg,
        }
    }

    /// CALCULATE-STRATEGY on integer regrets.
    fn strategy(&self, off: usize, n: usize, out: &mut [f64]) {
        let mut r = [0.0; MAX_ACTIONS];
        for a in 0..n {
            r[a] = self.regret[off + a] as f64;
        }
        regret_match(&r[..n], out);
    }

    /// UPDATE-STRATEGY(h, P_i).
    fn update_strategy(&mut self, id: u32, p: usize) {
        let tree = self.tree;
        let n = &tree.nodes[id as usize];
        if n.kind == Kind::Terminal
            || n.active >> p & 1 == 0
            || n.round as usize >= self.cfg.avg_rounds
        {
            return;
        }
        match n.kind {
            Kind::Chance => {
                let a = sample(&mut self.rng, tree.kid_probs(n));
                self.update_strategy(tree.kids(n)[a], p);
            }
            Kind::Player if n.player as usize == p => {
                let i = &tree.info[n.infoset as usize];
                let mut s = [0.0; MAX_ACTIONS];
                self.strategy(i.offset, i.n_actions, &mut s);
                let a = sample(&mut self.rng, &s[..i.n_actions]);
                self.phi[i.offset + a] += 1.0;
                self.update_strategy(tree.kids(n)[a], p);
            }
            _ => {
                for &c in tree.kids(n) {
                    self.update_strategy(c, p);
                }
            }
        }
    }

    /// TRAVERSE-MCCFR (`prune = false`) / TRAVERSE-MCCFR-P (`prune = true`).
    fn traverse(&mut self, id: u32, p: usize, prune: bool) -> f64 {
        let tree = self.tree;
        let n = &tree.nodes[id as usize];
        match n.kind {
            Kind::Terminal => return tree.utility(n, p),
            _ if n.active >> p & 1 == 0 => return self.traverse(tree.kids(n)[0], p, prune),
            Kind::Chance => {
                let a = sample(&mut self.rng, tree.kid_probs(n));
                return self.traverse(tree.kids(n)[a], p, prune);
            }
            Kind::Player => {}
        }
        let i = &tree.info[n.infoset as usize];
        let (off, na) = (i.offset, i.n_actions);
        let mut sigma = [0.0; MAX_ACTIONS];
        self.strategy(off, na, &mut sigma[..na]);
        if i.player != p {
            let a = sample(&mut self.rng, &sigma[..na]);
            return self.traverse(tree.kids(n)[a], p, prune);
        }
        let last_round = n.round as usize + 1 >= tree.n_rounds;
        let (mut vals, mut explored) = ([0.0; MAX_ACTIONS], [false; MAX_ACTIONS]);
        let mut v = 0.0;
        for (a, &c) in tree.kids(n).iter().enumerate() {
            let skip = prune
                && !last_round
                && self.regret[off + a] <= self.cfg.prune_threshold
                && tree.nodes[c as usize].kind != Kind::Terminal;
            if skip {
                self.pruned += 1;
            } else {
                vals[a] = self.traverse(c, p, prune);
                explored[a] = true;
                v += sigma[a] * vals[a];
            }
        }
        for a in 0..na {
            if explored[a] {
                let d = ((vals[a] - v) * self.cfg.utility_scale).round() as i64;
                let r = (self.regret[off + a] as i64 + d)
                    .clamp(self.cfg.regret_floor as i64, i32::MAX as i64);
                self.regret[off + a] = r as i32;
            }
        }
        v
    }
}

impl Solver for Mccfr<'_> {
    fn iterate(&mut self) {
        self.t += 1;
        let t = self.t;
        for p in 0..self.tree.n_players {
            if t % self.cfg.strategy_interval == 0 {
                self.update_strategy(Tree::ROOT, p);
            }
            let prune = t > self.cfg.prune_after && self.rng.f64() >= 1.0 - self.cfg.prune_prob;
            self.traverse(Tree::ROOT, p, prune);
        }
        if t < self.cfg.lcfr_until && t % self.cfg.discount_interval == 0 {
            let k = (t / self.cfg.discount_interval) as f64;
            let d = k / (k + 1.0);
            for r in &mut self.regret {
                *r = (*r as f64 * d).round() as i32;
            }
            self.phi.iter_mut().for_each(|x| *x *= d);
        }
    }
    fn iterations(&self) -> u64 {
        self.t
    }
    fn average(&self) -> Profile {
        self.tree.normalise(&self.phi)
    }
    fn current(&self) -> Profile {
        let mut out = vec![0.0; self.tree.total_actions];
        for i in &self.tree.info {
            self.strategy(
                i.offset,
                i.n_actions,
                &mut out[i.offset..i.offset + i.n_actions],
            );
        }
        out
    }
}
