//! Read access to the blueprint for search: action translation, per-hand strategy lookups
//! (belief updates, fallbacks) and the four continuation strategies used at depth-limit
//! leaves (supplementary "Leaf node values in imperfect-information subgames").
//!
//! The blueprint keys infosets by `(abstract-action history, card bucket)`. Real play can
//! leave the blueprint's action abstraction, so histories are built by mapping each real raise
//! to the nearest blueprint size with deterministic pseudo-harmonic translation (the paper maps
//! off-tree leaf nodes the same way).

use bench::bp::Source;
use bench::policy::Dist;
use blueprint::store::Store;
use blueprint::trainer::Bucketer;
use cards::{Card, Rng};
use nlhe::abstraction::{ActionList, BetSizing, Shadow, TrackOpts, MAX_ACTIONS};
use nlhe::{Action, State, Street};
use std::sync::Arc;

/// Blueprint access. History tracking is the bench policies' "shadow game"
/// ([`nlhe::abstraction::Shadow`], see `arena/bench/TRANSLATION.md`): every blueprint read in
/// search takes a `Shadow` (a copy of the game in which each off-menu raise was replaced by its
/// translated abstract action) plus the real state. Action lists, history key and node identity
/// come from the shadow, so lookups always hit a node of the blueprint's own tree. Reads fall
/// back (no prior / uniform likelihood / check-call) when the shadow lost sync with the game.
#[derive(Clone)]
pub struct Blueprint {
    pub store: Arc<Store>,
    /// Optional non-`Store` backend (e.g. a compact policy file); then `store` is a placeholder.
    pub source: Option<Arc<dyn Source>>,
    pub bucketer: Arc<dyn Bucketer + Send>,
    pub sizing: BetSizing,
    /// Shadow-tracking options (`TrackOpts::default()` = the fixed behaviour).
    pub opts: TrackOpts,
}

/// Continuation strategy ids: unaltered blueprint, then fold / call / raise biased x`bias`.
pub const K_BLUEPRINT: u8 = 0;
pub const K_FOLD: u8 = 1;
pub const K_CALL: u8 = 2;
pub const K_RAISE: u8 = 3;
pub const NUM_CONT: usize = 4;

impl Blueprint {
    pub fn new(store: Arc<Store>, bucketer: Arc<dyn Bucketer + Send>, sizing: BetSizing) -> Self {
        Blueprint { store, source: None, bucketer, sizing, opts: TrackOpts::default() }
    }

    /// Blueprint backed by a final-strategy [`Source`] (compact file) instead of a `Store`.
    pub fn from_source(source: Arc<dyn Source>, bucketer: Arc<dyn Bucketer + Send>, sizing: BetSizing) -> Self {
        let mut b = Self::new(Arc::new(Store::new(16)), bucketer, sizing);
        b.source = Some(source);
        b
    }

    pub fn with_opts(mut self, o: TrackOpts) -> Self {
        self.opts = o;
        self
    }

    /// Shadow game starting at `root` with empty history.
    pub fn shadow(&self, root: &State) -> Shadow {
        Shadow::new(root, self.opts)
    }

    /// Record real action `a` (taken at `pre`) in `sh`. `own`: our own action (maps back to the
    /// nearest list entry instead of being pseudo-harmonically translated).
    pub fn observe(&self, sh: &mut Shadow, pre: &State, a: Action, own: bool) {
        // Deterministic translation never draws; the generator is a required placeholder.
        let mut rng = Rng::new(0);
        sh.observe(pre, &self.sizing, a, own, &mut rng);
    }

    /// Blueprint action list of the shadow's node. `false` if the shadow cannot follow `real`.
    pub fn node_list(&self, sh: &Shadow, real: &State, out: &mut ActionList) -> bool {
        if sh.sync_reason(real) != 0 {
            return false;
        }
        sh.actions(real, &self.sizing, out);
        true
    }

    /// Strategy of the infoset `(shadow node, bucket)`; `list` must be `node_list`'s. `false`
    /// when the shadow is desynced or the infoset was never visited in training. Preflop uses
    /// the average strategy φ (store backend), postflop the final iterate, as in the paper.
    pub fn sigma(&self, sh: &Shadow, real: &State, list: &ActionList, bucket: u32, out: &mut [f32]) -> bool {
        if sh.sync_reason(real) != 0 {
            return false;
        }
        let n = list.len();
        let key = sh.key();
        if let Some(src) = &self.source {
            return src.probs(key, bucket, &mut out[..n]);
        }
        match self.store.lookup(key, bucket).filter(|s| s.n_actions as usize == n) {
            Some(slot) => {
                if real.street() == Street::Preflop {
                    self.store.average(slot, &mut out[..n]);
                } else {
                    self.store.strategy(slot, &mut out[..n]);
                }
                true
            }
            None => false,
        }
    }

    /// Blueprint action probabilities mapped onto a (different) search action list: each
    /// search action takes the translated blueprint action's probability, split equally among
    /// the search actions that translate to the same blueprint action.
    pub fn prior(&self, sh: &Shadow, st: &State, search: &ActionList, bucket: u32) -> Option<[f32; MAX_ACTIONS]> {
        let mut blist = ActionList::new();
        if !self.node_list(sh, st, &mut blist) {
            return None;
        }
        let mut bs = [0f32; MAX_ACTIONS];
        if !self.sigma(sh, st, &blist, bucket, &mut bs) {
            return None;
        }
        let mut idx = [0usize; MAX_ACTIONS];
        let mut cnt = [0u32; MAX_ACTIONS];
        for (i, a) in search.as_slice().iter().enumerate() {
            idx[i] = self.translate_in(sh, st, &blist, *a, false);
            cnt[idx[i]] += 1;
        }
        let mut out = [0f32; MAX_ACTIONS];
        for i in 0..search.len() {
            out[i] = bs[idx[i]] / cnt[idx[i]] as f32;
        }
        Some(out)
    }

    /// Index in `list` (the shadow node's list) of real action `a` taken at `pre`.
    pub fn translate_in(&self, sh: &Shadow, pre: &State, list: &ActionList, a: Action, own: bool) -> usize {
        let mut rng = Rng::new(0);
        sh.translate(pre, list, a, own, &mut rng).0
    }

    pub fn bucket(&self, street: Street, hole: [Card; 2], board: &[Card]) -> u32 {
        self.bucketer.bucket(street, hole, board)
    }

    /// Blueprint distribution over real actions for `hole` (check/call if unvisited or desynced).
    pub fn dist(&self, st: &State, sh: &Shadow, hole: [Card; 2], board: &[Card]) -> Dist {
        let mut d = Dist::new();
        let mut list = ActionList::new();
        if self.node_list(sh, st, &mut list) {
            let n = list.len();
            let bucket = self.bucket(st.street(), hole, board);
            let mut p = [0f32; MAX_ACTIONS];
            if self.sigma(sh, st, &list, bucket, &mut p) {
                for i in 0..n {
                    d.push(sh.real_action(st, &list, i), p[i]);
                }
                return d;
            }
        }
        d.push(real_passive(st), 1.0);
        d
    }
}

/// Check if legal, else call.
pub fn real_passive(st: &State) -> Action {
    if st.legal().can_check {
        Action::Check
    } else {
        Action::Call
    }
}

/// Position of the Check/Call entry.
pub fn passive_index(list: &ActionList) -> usize {
    list.as_slice().iter().position(|a| matches!(a, Action::Check | Action::Call)).unwrap()
}

/// Bias `sigma` for continuation strategy `k` (multiply fold / call / all raise probabilities
/// by `bias`, renormalise) and pick the action for uniform `u`.
pub fn cont_pick(k: u8, bias: f32, list: &ActionList, sigma: &mut [f32], u: f64) -> usize {
    if k != K_BLUEPRINT {
        for (i, a) in list.as_slice().iter().enumerate() {
            let hit = matches!(
                (k, a),
                (K_FOLD, Action::Fold) | (K_CALL, Action::Call | Action::Check) | (K_RAISE, Action::RaiseTo(_))
            );
            if hit {
                sigma[i] *= bias;
            }
        }
    }
    let total: f32 = sigma.iter().sum();
    if total <= 0.0 {
        return passive_index(list);
    }
    let mut x = u as f32 * total;
    for (i, &p) in sigma.iter().enumerate() {
        if x < p {
            return i;
        }
        x -= p;
    }
    sigma.len() - 1
}

impl crate::vector::Bucketing for Blueprint {
    fn bucket(&self, street: Street, hole: [Card; 2], board: &[Card]) -> u32 {
        self.bucketer.bucket(street, hole, board)
    }
    fn n_buckets(&self, street: Street) -> usize {
        self.bucketer.n_buckets(street) as usize
    }
}
