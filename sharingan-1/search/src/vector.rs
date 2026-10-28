//! Vector-form Linear CFR for heads-up subgames (supplement: "an optimized vector-based form of
//! Linear CFR ... that samples one set of public board cards per thread"), plus an exact best
//! response / exploitability evaluator on the same tree.
//!
//! Instead of sampling one pair of hole hands per traversal (`solver.rs`, external sampling) a
//! pass walks the betting tree ONCE with a reach vector over all live hole pairs per player
//! and returns counterfactual-value vectors; card removal at terminals is O(M) by
//! inclusion-exclusion over the two hole cards. Public chance (the next board card) is sampled,
//! stratified by cycling a shuffled deck per chance node, so a node below chance node `c` is
//! visited every ~k-th pass. Infosets are `(public node incl. board cards, exact hole pair)`:
//! no card abstraction anywhere in the subgame.
//!
//! Utilities: dead money from players who folded before the root is respected (winner takes
//! the pot), so the game is constant-sum (`u0 + u1 = dead`), not zero-sum.
//!
//! Conventions: player index 0/1 = `seats[0]/seats[1]`. Values are chips relative to the start
//! of the hand. Chance weights: sampling one of `k` unseen cards with weight `k/(k-4)` and
//! enumerating with weight `1/(k-4)` both give the conditional expectation over the `k-4`
//! cards not blocked by the two holes.

use crate::beliefs::holes;
use crate::tree::{search_actions, step_path, Extras};
use bench::mix;
use cards::{bit, evaluate, mask_of, Card, Rng};
use nlhe::abstraction::{ActionList, BetSizing, MAX_ACTIONS};
use nlhe::{Action, State, Step, Street};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

pub const N: usize = 1326;

/// Live hole pairs of the subgame (positive root reach for either player), compacted.
pub struct Hands {
    pub idx: Vec<u16>,
    pub cards: Vec<[u8; 2]>,
    /// Hole index (0..1326) -> compact index, `u32::MAX` if absent.
    pub of_hole: Vec<u32>,
    /// Compact indices of the hands containing card `c`.
    pub by_card: Vec<Vec<u32>>,
}

impl Hands {
    fn new(ranges: &[Vec<f64>; 2]) -> Hands {
        let hl = holes();
        let mut idx = Vec::new();
        let mut cards = Vec::new();
        let mut of_hole = vec![u32::MAX; N];
        let mut by_card = vec![Vec::new(); 52];
        for i in 0..N {
            if ranges[0][i] > 0.0 || ranges[1][i] > 0.0 {
                of_hole[i] = idx.len() as u32;
                for &c in &hl[i] {
                    by_card[c as usize].push(idx.len() as u32);
                }
                idx.push(i as u16);
                cards.push(hl[i]);
            }
        }
        Hands { idx, cards, of_hole, by_card }
    }
    pub fn len(&self) -> usize {
        self.idx.len()
    }
    pub fn is_empty(&self) -> bool {
        self.idx.is_empty()
    }
}

/// Card abstraction for the rounds after the root round (pooled infosets shared by every board
/// runout, exactly like the blueprint's keys).
pub trait Bucketing {
    fn bucket(&self, street: Street, hole: [Card; 2], board: &[Card]) -> u32;
    fn n_buckets(&self, street: Street) -> usize;
}

/// A heads-up subgame instance.
pub struct VecGame<'a> {
    pub root: State,
    /// Board visible at the root (3, 4 or 5 cards).
    pub board: &'a [Card],
    /// Player index -> seat.
    pub seats: [usize; 2],
    /// Root reach per player over all 1326 holes (must already exclude board-blocked hands).
    pub ranges: [Vec<f64>; 2],
    pub sizing: &'a BetSizing,
    pub later: &'a BetSizing,
    pub extras: &'a Extras,
    /// Our player index, our actual hole index and the actions we took already this round.
    pub me: Option<usize>,
    pub my_hole: Option<usize>,
    pub frozen: &'a HashMap<u64, Action>,
    /// `Some`: rounds after the root round use bucket-pooled infosets (regrets aggregated over
    /// the hands of a bucket weighted by the root belief); the root round stays lossless.
    pub pooled: Option<&'a dyn Bucketing>,
}

impl<'a> VecGame<'a> {
    /// Both live seats of `root` (None unless exactly two are live and the state is a decision).
    pub fn live_seats(root: &State) -> Option<[usize; 2]> {
        if root.is_terminal() || root.live_count() != 2 {
            return None;
        }
        let v: Vec<usize> = (0..root.num_players()).filter(|&s| !root.is_folded(s)).collect();
        Some([v[0], v[1]])
    }
}

/// Per-board showdown data over the compact hand list.
struct Ctx {
    valid: Vec<bool>,
    order: Vec<u32>,
    groups: Vec<(usize, usize)>,
}

impl Ctx {
    fn new(hands: &Hands, board: &[Card]) -> Ctx {
        let bm = mask_of(board);
        let m = hands.len();
        let mut valid = vec![false; m];
        let mut rank = vec![0u32; m];
        for i in 0..m {
            let h = hands.cards[i];
            if bit(h[0]) & bm == 0 && bit(h[1]) & bm == 0 {
                valid[i] = true;
                rank[i] = evaluate(bit(h[0]) | bit(h[1]) | bm);
            }
        }
        let mut order: Vec<u32> = (0..m as u32).filter(|&i| valid[i as usize]).collect();
        order.sort_by_key(|&i| rank[i as usize]);
        let mut groups = Vec::new();
        let mut s = 0;
        for i in 1..=order.len() {
            if i == order.len() || rank[order[i] as usize] != rank[order[s] as usize] {
                groups.push((s, i));
                s = i;
            }
        }
        Ctx { valid, order, groups }
    }
}

fn card_sums(hands: &Hands, r: &[f64]) -> (f64, [f64; 52]) {
    let mut t = 0.0;
    let mut c = [0.0; 52];
    for i in 0..r.len() {
        let x = r[i];
        if x != 0.0 {
            t += x;
            c[hands.cards[i][0] as usize] += x;
            c[hands.cards[i][1] as usize] += x;
        }
    }
    (t, c)
}

/// `out[i]` = sum of `r[j]` over hands `j` sharing no card with hand `i`.
fn compat(hands: &Hands, r: &[f64]) -> Vec<f64> {
    let (t, c) = card_sums(hands, r);
    (0..r.len())
        .map(|i| {
            let [a, b] = hands.cards[i];
            t - c[a as usize] - c[b as usize] + r[i]
        })
        .collect()
}

/// Showdown cfv of the holder: `win*P(>) - lose*P(<) + tie*P(=)` weighted by opponent reach.
fn showdown(hands: &Hands, ctx: &Ctx, r: &[f64], win: f64, lose: f64, tie: f64) -> Vec<f64> {
    let (tot, tc) = card_sums(hands, r);
    let mut less = 0.0;
    let mut lc = [0.0f64; 52];
    let mut out = vec![0.0; r.len()];
    for &(s, e) in &ctx.groups {
        if e - s == 1 {
            // Singleton tie group: no equal-rank hands to account for.
            let h = ctx.order[s] as usize;
            let [a, b] = [hands.cards[h][0] as usize, hands.cards[h][1] as usize];
            let comp_less = less - lc[a] - lc[b];
            let comp_all = tot - tc[a] - tc[b] + r[h];
            out[h] = win * comp_less - lose * (comp_all - comp_less);
            less += r[h];
            lc[a] += r[h];
            lc[b] += r[h];
            continue;
        }
        let mut gt = 0.0;
        let mut gc = [0.0f64; 52];
        for &h in &ctx.order[s..e] {
            let h = h as usize;
            gt += r[h];
            gc[hands.cards[h][0] as usize] += r[h];
            gc[hands.cards[h][1] as usize] += r[h];
        }
        for &h in &ctx.order[s..e] {
            let h = h as usize;
            let [a, b] = [hands.cards[h][0] as usize, hands.cards[h][1] as usize];
            let comp_less = less - lc[a] - lc[b];
            let comp_eq = gt - gc[a] - gc[b] + r[h];
            let comp_all = tot - tc[a] - tc[b] + r[h];
            let greater = comp_all - comp_less - comp_eq;
            out[h] = win * comp_less - lose * greater + tie * comp_eq;
        }
        for &h in &ctx.order[s..e] {
            let h = h as usize;
            less += r[h];
            lc[hands.cards[h][0] as usize] += r[h];
            lc[hands.cards[h][1] as usize] += r[h];
        }
    }
    out
}

struct Perm {
    order: Vec<Card>,
    pos: usize,
}

/// State shared by the solver and the best-response evaluator.
struct Core<'a> {
    g: &'a VecGame<'a>,
    hands: Hands,
    board: Vec<Card>,
    ctxs: HashMap<u64, Rc<Ctx>>,
    perms: HashMap<u64, Perm>,
    path: Vec<Action>,
    rng: Rng,
    enumerate: bool,
    stratify: bool,
    dead: f64,
    root_street: Street,
    bk_cache: HashMap<(u64, u8), Rc<Vec<u32>>>,
    /// Root belief per player over the compact hands (regret weights of pooled infosets).
    w: [Vec<f64>; 2],
}

impl<'a> Core<'a> {
    fn new(g: &'a VecGame<'a>, seed: u64, enumerate: bool, stratify: bool) -> Core<'a> {
        let c0 = g.root.contributed(g.seats[0]) + g.root.contributed(g.seats[1]);
        let hands = Hands::new(&g.ranges);
        let w = [0, 1].map(|p| hands.idx.iter().map(|&i| g.ranges[p][i as usize]).collect());
        Core {
            g,
            w,
            bk_cache: HashMap::new(),
            hands,
            board: g.board.to_vec(),
            ctxs: HashMap::new(),
            perms: HashMap::new(),
            path: Vec::new(),
            rng: Rng::new(seed),
            enumerate,
            stratify,
            dead: (g.root.pot() - c0) as f64,
            root_street: g.root.street(),
        }
    }

    /// Bucket of every compact hand on the current board and `street`.
    fn buckets(&mut self, street: Street) -> Rc<Vec<u32>> {
        let key = (mask_of(&self.board), street.idx() as u8);
        if let Some(b) = self.bk_cache.get(&key) {
            return b.clone();
        }
        let pb = self.g.pooled.unwrap();
        let hl = holes();
        let bm = key.0;
        // Hands blocked by the board are impossible (zero reach): any bucket will do.
        let v: Vec<u32> = self
            .hands
            .idx
            .iter()
            .map(|&i| {
                let h = hl[i as usize];
                if (bit(h[0]) | bit(h[1])) & bm != 0 {
                    0
                } else {
                    pb.bucket(street, h, &self.board)
                }
            })
            .collect();
        let v = Rc::new(v);
        self.bk_cache.insert(key, v.clone());
        v
    }

    fn player_of(&self, seat: usize) -> usize {
        (seat != self.g.seats[0]) as usize
    }

    fn ctx(&mut self) -> Rc<Ctx> {
        let key = mask_of(&self.board);
        if let Some(c) = self.ctxs.get(&key) {
            return c.clone();
        }
        let c = Rc::new(Ctx::new(&self.hands, &self.board));
        self.ctxs.insert(key, c.clone());
        c
    }

    /// Cards to branch on at a chance node with identity `key`, each with its weight.
    fn deal(&mut self, key: u64) -> Vec<(Card, f64)> {
        let used = mask_of(&self.board);
        let free: Vec<Card> = (0..52u8).filter(|&c| used & bit(c) == 0).collect();
        let k = free.len() as f64;
        if self.enumerate {
            return free.iter().map(|&c| (c, 1.0 / (k - 4.0))).collect();
        }
        let c = if self.stratify {
            let p = self.perms.entry(key).or_insert(Perm { order: vec![], pos: 0 });
            if p.pos >= p.order.len() {
                p.order = free.clone();
                self.rng.shuffle(&mut p.order);
                p.pos = 0;
            }
            p.pos += 1;
            p.order[p.pos - 1]
        } else {
            free[self.rng.below(free.len() as u32) as usize]
        };
        vec![(c, k / (k - 4.0))]
    }

    /// Terminal cfv vectors for both players: folds, or showdown (running out the board).
    fn terminal(&mut self, st: &State, reach: [&[f64]; 2], key: u64) -> [Vec<f64>; 2] {
        let g = self.g;
        if !st.showdown() {
            let [s0, s1] = g.seats;
            let f = if st.is_folded(s0) { 0 } else { 1 };
            let w = 1 - f;
            let cf = st.contributed(g.seats[f]) as f64;
            let mut vf = compat(&self.hands, reach[w]);
            let mut vw = compat(&self.hands, reach[f]);
            vf.iter_mut().for_each(|x| *x *= -cf);
            vw.iter_mut().for_each(|x| *x *= self.dead + cf);
            let _ = (s0, s1);
            let mut out = [Vec::new(), Vec::new()];
            out[f] = vf;
            out[w] = vw;
            return out;
        }
        if self.board.len() < 5 {
            let m = self.hands.len();
            let mut out = [vec![0.0; m], vec![0.0; m]];
            for (c, w) in self.deal(mix(key, 0x7E4)) {
                let r0 = self.masked(reach[0], c);
                let r1 = self.masked(reach[1], c);
                self.board.push(c);
                let mut v = self.terminal(st, [&r0, &r1], mix(key, c as u64 + 1));
                self.board.pop();
                for p in 0..2 {
                    for &h in &self.hands.by_card[c as usize] {
                        v[p][h as usize] = 0.0;
                    }
                    for i in 0..m {
                        out[p][i] += w * v[p][i];
                    }
                }
            }
            return out;
        }
        let ctx = self.ctx();
        let c = st.contributed(g.seats[0]).min(st.contributed(g.seats[1])) as f64;
        let (win, lose, tie) = (self.dead + c, c, self.dead / 2.0);
        let mut v0 = showdown(&self.hands, &ctx, reach[1], win, lose, tie);
        let mut v1 = showdown(&self.hands, &ctx, reach[0], win, lose, tie);
        for i in 0..v0.len() {
            if !ctx.valid[i] {
                v0[i] = 0.0;
                v1[i] = 0.0;
            }
        }
        [v0, v1]
    }

    fn masked(&self, r: &[f64], c: Card) -> Vec<f64> {
        let mut v = r.to_vec();
        for &h in &self.hands.by_card[c as usize] {
            v[h as usize] = 0.0;
        }
        v
    }

    fn actions(&self, st: &State, akey: u64, list: &mut ActionList) {
        if st.street() == self.root_street {
            let ex: &[i32] = self.g.extras.get(&akey).map_or(&[], |v| v.as_slice());
            search_actions(st, self.g.sizing, ex, list);
        } else {
            search_actions(st, self.g.later, &[], list);
        }
    }
}

struct VNode {
    n: usize,
    reg: Vec<f64>,
    ssum: Vec<f64>,
    root: bool,
    player: u8,
    /// Bucket count for pooled nodes (0 = per-hand node).
    nb: usize,
}

/// Solution of a vector solve: the root round's strategies per public node and hole pair.
pub struct VecSolution {
    nodes: HashMap<u64, VNode>,
    hands: Hands,
    pub iters: u64,
    pub millis: f64,
    /// Bytes of regret / average storage allocated during the solve (peak, before dropping later rounds).
    pub peak_bytes: usize,
    /// Root reach per compact hand (belief) and `sum_t t`: the average strategy of a hand whose
    /// accumulated own-path weight is a tiny fraction of the maximum is meaningless (the node is
    /// practically never reached with it) and `probs` reports it as unavailable.
    root_w: [Vec<f64>; 2],
    total_t: f64,
    root_street: Street,
}

/// Minimum average own-path probability for the averaged strategy of a hand to be trusted.
const MIN_REACH: f64 = 1e-3;

impl VecSolution {
    /// Strategy of root-round node `path` for hole pair `hole` (index 0..1326): current regret
    /// matching (`final_iterate`) or the weighted average. `false` if not available.
    pub fn probs(&self, path: u64, hole: usize, n: usize, final_iterate: bool, out: &mut [f32]) -> bool {
        let Some(nd) = self.nodes.get(&path).filter(|nd| nd.n == n && nd.root) else { return false };
        let m = self.hands.of_hole[hole];
        if m == u32::MAX {
            return false;
        }
        let (m, mm) = (m as usize, self.hands.len());
        if final_iterate {
            let tot: f64 = (0..n).map(|a| nd.reg[a * mm + m].max(0.0)).sum();
            for a in 0..n {
                out[a] = if tot > 0.0 { (nd.reg[a * mm + m].max(0.0) / tot) as f32 } else { 1.0 / n as f32 };
            }
            true
        } else {
            let tot: f64 = (0..n).map(|a| nd.ssum[a * mm + m]).sum();
            if tot <= 0.0 || tot < MIN_REACH * self.root_w[nd.player as usize][m] * self.total_t {
                return false;
            }
            for a in 0..n {
                out[a] = (nd.ssum[a * mm + m] / tot) as f32;
            }
            true
        }
    }
    pub fn nodes(&self) -> usize {
        self.nodes.len()
    }
    pub fn hands(&self) -> &Hands {
        &self.hands
    }
    /// Full-subgame strategy (needs `keep_all` at solve time) for exploitability evaluation.
    pub fn strategy<'b>(&'b self, average: bool, pooled: Option<&'b dyn Bucketing>) -> impl Strat + 'b {
        SolStrat { sol: self, average, pooled }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct VecParams {
    pub iters: u64,
    pub max_millis: Option<u64>,
    /// CFR+ style (regrets floored at 0, unweighted) instead of Linear CFR.
    pub plus: bool,
    pub stratify: bool,
    pub seed: u64,
    /// Keep every street's strategies (default: root round only, to save memory).
    pub keep_all: bool,
    /// Stop the solve (keeping the passes done) once node storage exceeds this many bytes.
    pub max_bytes: usize,
}

impl Default for VecParams {
    fn default() -> Self {
        VecParams { iters: 300, max_millis: None, plus: false, stratify: true, seed: 1, keep_all: false, max_bytes: 3 << 30 }
    }
}

struct Solver<'a> {
    core: Core<'a>,
    nodes: HashMap<u64, VNode>,
    t: f64,
    plus: bool,
    frozen_m: Option<usize>,
    bytes: usize,
}

pub fn solve_vec(g: &VecGame, p: &VecParams) -> VecSolution {
    let start = Instant::now();
    let core = Core::new(g, p.seed, false, p.stratify);
    let frozen_m = g.my_hole.and_then(|h| match core.hands.of_hole[h] {
        u32::MAX => None,
        m => Some(m as usize),
    });
    let mut sv = Solver { core, nodes: HashMap::new(), t: 0.0, plus: p.plus, frozen_m, bytes: 0 };
    let r0: Vec<f64> = sv.core.hands.idx.iter().map(|&i| g.ranges[0][i as usize]).collect();
    let r1: Vec<f64> = sv.core.hands.idx.iter().map(|&i| g.ranges[1][i as usize]).collect();
    let root = g.root;
    let mut done = 0;
    if !root.is_terminal() && !sv.core.hands.is_empty() {
        for t in 1..=p.iters {
            sv.t = t as f64;
            sv.walk(&root, 0, 0, [&r0, &r1]);
            done = t;
            if sv.bytes > p.max_bytes {
                break;
            }
            if p.max_millis.is_some_and(|ms| start.elapsed().as_millis() as u64 >= ms) {
                break;
            }
        }
    }
    let sv_bytes = sv.bytes;
    let mut nodes = sv.nodes;
    if !p.keep_all {
        nodes.retain(|_, v| v.root);
    }
    let root_w = [0, 1].map(|p| sv.core.hands.idx.iter().map(|&i| g.ranges[p][i as usize]).collect());
    let total_t = done as f64 * (done as f64 + 1.0) / 2.0;
    VecSolution { nodes, hands: sv.core.hands, iters: done, millis: start.elapsed().as_secs_f64() * 1e3, peak_bytes: sv_bytes, root_w, total_t, root_street: g.root.street() }
}

fn finish(p: usize, q: usize, vp: Vec<f64>, vq: Vec<f64>) -> [Vec<f64>; 2] {
    let mut out = [Vec::new(), Vec::new()];
    out[p] = vp;
    out[q] = vq;
    out
}

impl<'a> Solver<'a> {
    fn walk(&mut self, st: &State, akey: u64, vkey: u64, reach: [&[f64]; 2]) -> [Vec<f64>; 2] {
        let g = self.core.g;
        let m = self.core.hands.len();
        let seat = st.to_act().unwrap();
        let p = self.core.player_of(seat);
        let q = 1 - p;
        let mut list = ActionList::new();
        self.core.actions(st, akey, &mut list);
        let n = list.len();
        let is_root = st.street() == self.core.root_street;
        let pooled = !is_root && g.pooled.is_some();
        let bk: Option<Rc<Vec<u32>>> = if pooled { Some(self.core.buckets(st.street())) } else { None };
        let nb = if pooled { g.pooled.unwrap().n_buckets(st.street()) } else { 0 };
        let nkey = if pooled { mix(akey, 0xB0C7) } else { vkey };
        let width = if pooled { nb } else { m };
        let mut fresh = 0usize;
        let _node = self.nodes.entry(nkey).or_insert_with(|| {
            fresh = 16 * n * width;
            VNode { n, reg: vec![0.0; n * width], ssum: vec![0.0; n * width], root: is_root, player: p as u8, nb }
        });
        self.bytes += fresh;
        let node = self.nodes.get_mut(&nkey).unwrap();
        debug_assert_eq!(node.n, n);
        // Regret matching (action-major loops over contiguous slices); pooled nodes first per
        // bucket, then expanded to the hands.
        let mut sigb = vec![0.0f64; n * width];
        let mut tot = vec![0.0f64; width];
        for a in 0..n {
            for (t, &r) in tot.iter_mut().zip(&node.reg[a * width..(a + 1) * width]) {
                *t += r.max(0.0);
            }
        }
        let uni = 1.0 / n as f64;
        for a in 0..n {
            for ((s, &r), &t) in sigb[a * width..(a + 1) * width].iter_mut().zip(&node.reg[a * width..(a + 1) * width]).zip(&tot) {
                *s = if t > 0.0 { r.max(0.0) / t } else { uni };
            }
        }
        let mut sig = if let Some(bk) = &bk {
            let mut v = vec![0.0f64; n * m];
            for a in 0..n {
                for (x, &b) in v[a * m..(a + 1) * m].iter_mut().zip(bk.iter()) {
                    *x = sigb[a * nb + b as usize];
                }
            }
            v
        } else {
            std::mem::take(&mut sigb)
        };
        // Our own earlier action this round is fixed for our actual hand.
        let mut frozen_h = None;
        let mut frozen_f = 0usize;
        if is_root && Some(p) == g.me {
            if let (Some(h), Some(fa)) = (self.frozen_m, g.frozen.get(&akey)) {
                if let Some(f) = list.as_slice().iter().position(|a| a == fa) {
                    for a in 0..n {
                        sig[a * m + h] = (a == f) as u8 as f64;
                    }
                    frozen_h = Some(h);
                    frozen_f = f;
                }
            }
        }
        let mut vp = vec![0.0; m];
        let mut vq = vec![0.0; m];
        let mut cps: Vec<Vec<f64>> = Vec::with_capacity(n);
        for a in 0..n {
            let act = list.get(a);
            let mut s2 = *st;
            let step = s2.apply(act);
            let nr: Vec<f64> = reach[p].iter().zip(&sig[a * m..(a + 1) * m]).map(|(r, s)| r * s).collect();
            let mut r2 = reach;
            r2[p] = &nr;
            let akey2 = step_path(akey, act);
            let vkey2 = step_path(vkey, act);
            let mut c = match step {
                Step::Terminal => self.core.terminal(&s2, r2, vkey2),
                Step::NewStreet(_) => self.chance(&s2, akey2, vkey2, r2),
                Step::Continue => self.walk(&s2, akey2, vkey2, r2),
            };
            for ((v, &s), &x) in vp.iter_mut().zip(&sig[a * m..(a + 1) * m]).zip(&c[p]) {
                *v += s * x;
            }
            for (v, &x) in vq.iter_mut().zip(&c[q]) {
                *v += x;
            }
            cps.push(std::mem::take(&mut c[p]));
        }
        let t = self.t;
        let plus = self.plus;
        let node = self.nodes.get_mut(&nkey).unwrap();
        if let Some(bk) = &bk {
            let w = &self.core.w[p];
            let mut rsum = vec![0.0f64; nb];
            for (&b, &r) in bk.iter().zip(reach[p]) {
                rsum[b as usize] += r;
            }
            let mut delta = vec![0.0f64; nb];
            for a in 0..n {
                delta.iter_mut().for_each(|x| *x = 0.0);
                for (((&b, &c), &v), &wt) in bk.iter().zip(&cps[a]).zip(&vp).zip(w) {
                    delta[b as usize] += wt * (c - v);
                }
                let (reg, ss) = (&mut node.reg[a * nb..(a + 1) * nb], &mut node.ssum[a * nb..(a + 1) * nb]);
                for b in 0..nb {
                    reg[b] = if plus { (reg[b] + delta[b]).max(0.0) } else { reg[b] + t * delta[b] };
                    ss[b] += t * rsum[b] * sigb[a * nb + b];
                }
            }
            return finish(p, q, vp, vq);
        }
        for a in 0..n {
            let (reg, ss) = (&mut node.reg[a * m..(a + 1) * m], &mut node.ssum[a * m..(a + 1) * m]);
            let (cp, sg) = (&cps[a], &sig[a * m..(a + 1) * m]);
            let keep = frozen_h.map(|h| (h, reg[h]));
            if plus {
                for (((r, &c), &v), (x, (&rp, &s))) in reg.iter_mut().zip(cp).zip(&vp).zip(ss.iter_mut().zip(reach[p].iter().zip(sg))) {
                    *r = (*r + c - v).max(0.0);
                    *x += t * rp * s;
                }
            } else {
                for (((r, &c), &v), (x, (&rp, &s))) in reg.iter_mut().zip(cp).zip(&vp).zip(ss.iter_mut().zip(reach[p].iter().zip(sg))) {
                    *r += t * (c - v);
                    *x += t * rp * s;
                }
            }
            if let Some((h, _)) = keep {
                // Readers of the final iterate see the pinned pure action.
                reg[h] = (a == frozen_f) as u8 as f64;
            }
        }
        finish(p, q, vp, vq)
    }

    /// A new street opens: branch on the next board card(s).
    fn chance(&mut self, st: &State, akey: u64, vkey: u64, reach: [&[f64]; 2]) -> [Vec<f64>; 2] {
        let m = self.core.hands.len();
        let mut out = [vec![0.0; m], vec![0.0; m]];
        for (c, w) in self.core.deal(vkey) {
            let r0 = self.core.masked(reach[0], c);
            let r1 = self.core.masked(reach[1], c);
            self.core.board.push(c);
            let mut v = self.walk(st, akey, mix(vkey, c as u64 + 1), [&r0, &r1]);
            self.core.board.pop();
            for p in 0..2 {
                for &h in &self.core.hands.by_card[c as usize] {
                    v[p][h as usize] = 0.0;
                }
                for i in 0..m {
                    out[p][i] += w * v[p][i];
                }
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------------------------
// Exact best response / exploitability
// ---------------------------------------------------------------------------------------------

/// Where in the subgame a strategy is queried.
pub struct Query<'a> {
    pub akey: u64,
    pub vkey: u64,
    pub street: Street,
    pub board: &'a [Card],
    pub seat: usize,
    pub hands: &'a Hands,
    /// Decision state and the actions taken since the subgame root.
    pub st: &'a State,
    pub path: &'a [Action],
}

/// A complete strategy profile: fill `out[a*M + m]` (M = number of compact hands) for the
/// acting player at the queried node.
pub trait Strat {
    fn probs(&self, q: &Query, n: usize, out: &mut [f64]);
}

struct SolStrat<'a> {
    sol: &'a VecSolution,
    average: bool,
    pooled: Option<&'a dyn Bucketing>,
}

impl Strat for SolStrat<'_> {
    fn probs(&self, q: &Query, n: usize, out: &mut [f64]) {
        let m = q.hands.len();
        let uni = |out: &mut [f64]| out.iter_mut().for_each(|x| *x = 1.0 / n as f64);
        let pooled = q.street != self.sol.root_street && self.pooled.is_some();
        let key = if pooled { mix(q.akey, 0xB0C7) } else { q.vkey };
        let Some(nd) = self.sol.nodes.get(&key).filter(|nd| nd.n == n) else { return uni(out) };
        let pick = |a: usize, w: usize| -> f64 { if self.average { nd.ssum[a * (if pooled { nd.nb } else { m }) + w] } else { nd.reg[a * (if pooled { nd.nb } else { m }) + w].max(0.0) } };
        let hl = holes();
        for h in 0..m {
            let w = if pooled { self.pooled.unwrap().bucket(q.street, hl[q.hands.idx[h] as usize], q.board) as usize } else { h };
            let tot: f64 = (0..n).map(|a| pick(a, w)).sum();
            for a in 0..n {
                out[a * m + h] = if tot > 0.0 { pick(a, w) / tot } else { 1.0 / n as f64 };
            }
        }
    }
}

/// Exact best-response values. Returns `(exploitability, [br0, br1], z)` in chips: the mean of
/// both players' best-response gains over the equilibrium value, normalised by the compatible
/// range mass.
pub fn exploitability(g: &VecGame, s: &dyn Strat) -> (f64, [f64; 2]) {
    let mut core = Core::new(g, 0, true, false);
    let m = core.hands.len();
    let r: [Vec<f64>; 2] = [0, 1].map(|p| core.hands.idx.iter().map(|&i| g.ranges[p][i as usize]).collect());
    let comp = compat(&core.hands, &r[1]);
    let z: f64 = (0..m).map(|h| r[0][h] * comp[h]).sum();
    let mut br = [0.0; 2];
    for p in 0..2 {
        let v = core.br(g.root, 0, 0, p, &r[1 - p], s);
        br[p] = (0..m).map(|h| r[p][h] * v[h]).sum::<f64>() / z;
    }
    let dead = core.dead;
    (((br[0] + br[1]) - dead) / 2.0, br)
}

impl<'a> Core<'a> {
    /// Best-response cfv of player `p` against `s` for the other player, given the opponent's reach.
    fn br(&mut self, st: State, akey: u64, vkey: u64, p: usize, opp: &[f64], s: &dyn Strat) -> Vec<f64> {
        let m = self.hands.len();
        let q = 1 - p;
        let seat = st.to_act().unwrap();
        let mut list = ActionList::new();
        self.actions(&st, akey, &mut list);
        let n = list.len();
        let actor = self.player_of(seat);
        let mut sig = vec![0.0f64; if actor == p { 0 } else { n * m }];
        if actor == q {
            let hands = std::mem::replace(&mut self.hands, Hands { idx: vec![], cards: vec![], of_hole: vec![], by_card: vec![] });
            {
                let board = self.board.clone();
                let qy = Query { akey, vkey, street: st.street(), board: &board, seat, hands: &hands, st: &st, path: &self.path };
                s.probs(&qy, n, &mut sig);
            }
            self.hands = hands;
        }
        let dummy = vec![0.0; m];
        let mut best = vec![f64::NEG_INFINITY; m];
        let mut sum = vec![0.0; m];
        for a in 0..n {
            let act = list.get(a);
            let mut s2 = st;
            let step = s2.apply(act);
            let nr: Vec<f64> = if actor == q { (0..m).map(|h| opp[h] * sig[a * m + h]).collect() } else { opp.to_vec() };
            let akey2 = step_path(akey, act);
            let vkey2 = step_path(vkey, act);
            self.path.push(act);
            let c = match step {
                Step::Terminal => {
                    let mut reach: [&[f64]; 2] = [&dummy, &dummy];
                    reach[q] = &nr;
                    let mut v = self.terminal(&s2, reach, vkey2);
                    std::mem::take(&mut v[p])
                }
                Step::NewStreet(_) => {
                    let mut acc = vec![0.0; m];
                    for (cd, w) in self.deal(vkey2) {
                        let nrm = self.masked(&nr, cd);
                        self.board.push(cd);
                        let mut v = self.br(s2, akey2, mix(vkey2, cd as u64 + 1), p, &nrm, s);
                        self.board.pop();
                        for &h in &self.hands.by_card[cd as usize] {
                            v[h as usize] = 0.0;
                        }
                        for i in 0..m {
                            acc[i] += w * v[i];
                        }
                    }
                    acc
                }
                Step::Continue => self.br(s2, akey2, vkey2, p, &nr, s),
            };
            self.path.pop();
            if actor == p {
                for h in 0..m {
                    best[h] = best[h].max(c[h]);
                }
            } else {
                for h in 0..m {
                    sum[h] += c[h];
                }
            }
        }
        if actor == p {
            best
        } else {
            sum
        }
    }
}

pub const _MAX: usize = MAX_ACTIONS;
