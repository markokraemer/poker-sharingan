//! Blueprint-backed policy, biased continuation variants, a coarse test bucketer, and
//! checkpoint loading (via `blueprint::io`, which stores only the `Store`; the bucketer and
//! sizing are chosen by name, see [`bucketer_by_name`] / [`sizing_by_name`]).

use crate::policy::{Dist, Player, PolicyFactory, View};
use blueprint::store::Store;
use blueprint::trainer::Bucketer;
use cards::{Card, Rng};
use nlhe::abstraction::{ActionList, BetSizing, Shadow};
pub use nlhe::abstraction::{BelowMenu, TrackOpts, Translation};
use nlhe::{Action, GameConfig, State, Street};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Arc;

/// Preflop lossless (169); postflop made-hand category only. For tests and toy blueprints.
pub struct CoarseBucketer;
impl Bucketer for CoarseBucketer {
    fn bucket(&self, street: Street, hole: [Card; 2], board: &[Card]) -> u32 {
        if street == Street::Preflop {
            return cards::preflop_class(hole[0], hole[1]) as u32;
        }
        let m = cards::mask_of(board) | cards::bit(hole[0]) | cards::bit(hole[1]);
        cards::category(cards::evaluate(m)) as u32
    }
    fn n_buckets(&self, street: Street) -> u32 {
        if street == Street::Preflop {
            169
        } else {
            9
        }
    }
}

/// Real abstraction: `abstraction::tables::BucketTables` (preflop 169 lossless, postflop k-means).
pub struct TableBucketer(pub abstraction::tables::BucketTables);
impl Bucketer for TableBucketer {
    fn bucket(&self, _street: Street, hole: [Card; 2], board: &[Card]) -> u32 {
        self.0.bucket(hole, board)
    }
    fn n_buckets(&self, street: Street) -> u32 {
        match street {
            Street::Preflop => 169,
            s => self.0.buckets[s.idx() - 1] as u32,
        }
    }
}

/// Registry hook: add real abstraction tables here when they exist.
pub fn bucketer_by_name(name: &str) -> Option<Arc<dyn Bucketer + Send>> {
    match name {
        "coarse" => Some(Arc::new(CoarseBucketer)),
        // "tables:<dir>", e.g. tables:data/buckets-200
        n if n.starts_with("tables:") => {
            let t = abstraction::tables::BucketTables::load(std::path::Path::new(&n[7..])).ok()?;
            Some(Arc::new(TableBucketer(t)))
        }
        _ => None,
    }
}

pub fn sizing_by_name(name: &str) -> Option<BetSizing> {
    match name {
        "tiny" => Some(BetSizing::tiny()),
        "pluribus" | "paper" => Some(BetSizing::pluribus_blueprint()),
        "local" => {
            use nlhe::abstraction::Menu;
            let m = Menu::new;
            let pre = [
                m(&[0.4, 0.6, 1.0, 2.0], true),
                m(&[0.75, 1.25], true),
                m(&[1.0], true),
                m(&[], true),
            ];
            let flop = [m(&[0.5, 1.0], true), m(&[1.0], true)];
            let late = [m(&[0.5, 1.0], true), m(&[1.0], true)];
            Some(BetSizing::new([&pre, &flop, &late, &late], [4, 3, 3, 3]))
        }
        "medium" => {
            // Between `local` and `paper`: 8 preflop opening sizes (~2–8.5bb), richer 3-bet and
            // flop menus; turn/river exactly as published.
            use nlhe::abstraction::Menu;
            let m = Menu::new;
            let pre = [
                m(&[0.4, 0.5, 0.6, 0.75, 1.0, 1.5, 2.0, 3.0], true),
                m(&[0.5, 0.75, 1.0, 1.5], true),
                m(&[0.75, 1.25], true),
                m(&[], true),
            ];
            let flop = [m(&[0.33, 0.5, 1.0], true), m(&[0.5, 1.0], true)];
            let late = [m(&[0.5, 1.0], true), m(&[1.0], true)];
            Some(BetSizing::new([&pre, &flop, &late, &late], [4, 3, 3, 3]))
        }
        "adversarial" => {
            // Attacker menu for exploitability tests against a `medium` target (never a target
            // itself): tiny bets and near-min raises, sizes just past the target's pseudo-harmonic
            // midpoints, overbets, and one more raise per street than the target's tree allows.
            use nlhe::abstraction::Menu;
            let m = Menu::new;
            let pre = [
                m(&[0.05, 0.15, 0.3, 0.45, 0.7, 1.0, 2.0], true),
                m(&[0.05, 0.3, 0.6, 1.0, 2.0], true),
                m(&[0.05, 0.5, 1.0], true),
                m(&[0.05], true),
                m(&[0.05], true),
            ];
            let post = [
                m(&[0.05, 0.15, 0.25, 0.42, 0.7, 1.3, 2.5], true),
                m(&[0.05, 0.4, 1.0], true),
                m(&[0.05], true),
                m(&[0.05], true),
            ];
            Some(BetSizing::new([&pre, &post, &post, &post], [5, 4, 4, 4]))
        }
        _ => None,
    }
}

/// Final-strategy lookup backend other than a live `Store` (e.g. a compact policy file).
/// Must return the *final* distribution (no street-dependent averaging left to do).
pub trait Source: Send + Sync {
    /// Fill `out` (one entry per abstract action) for (history key, bucket); false if the
    /// infoset is absent or has a different action count.
    fn probs(&self, hist: u64, bucket: u32, out: &mut [f32]) -> bool;
}

struct Shared {
    /// Placeholder (empty) store when `source` is set.
    store: Arc<Store>,
    source: Option<Arc<dyn Source>>,
    bucketer: Arc<dyn Bucketer + Send>,
    sizing: BetSizing,
    opts: TrackOpts,
    avg_preflop: bool,
    misses: AtomicU64,
    lookups: AtomicU64,
    /// Decision diagnostics split by whether an earlier action was off-menu: `[on, off]`.
    diag: [Diag; 2],
}

/// Per-class decision counters; probability masses are in micro-units.
#[derive(Default)]
struct Diag {
    lookups: AtomicU64,
    misses: AtomicU64,
    desync: AtomicU64,
    /// Desync reasons (index = `Shadow::sync_reason`).
    why: [AtomicU64; 6],
    fold: AtomicU64,
    call: AtomicU64,
    raise: AtomicU64,
}

pub struct BlueprintPolicy {
    name: String,
    sh: Arc<Shared>,
}

impl BlueprintPolicy {
    pub fn new(
        name: &str,
        store: Arc<Store>,
        bucketer: Arc<dyn Bucketer + Send>,
        sizing: BetSizing,
    ) -> Self {
        BlueprintPolicy {
            name: name.to_string(),
            sh: Arc::new(Shared {
                store,
                source: None,
                bucketer,
                sizing,
                opts: TrackOpts::default(),
                avg_preflop: true,
                misses: AtomicU64::new(0),
                lookups: AtomicU64::new(0),
                diag: Default::default(),
            }),
        }
    }
    /// Policy over a non-`Store` backend ([`Source`]): same history tracking, translation and
    /// fallback as the store-backed policy. `store()` then returns an empty placeholder.
    pub fn from_source(
        name: &str,
        source: Arc<dyn Source>,
        bucketer: Arc<dyn Bucketer + Send>,
        sizing: BetSizing,
    ) -> Self {
        let mut p = Self::new(name, Arc::new(Store::new(16)), bucketer, sizing);
        Arc::get_mut(&mut p.sh).expect("unshared").source = Some(source);
        p
    }
    pub fn with_translation(mut self, t: Translation) -> Self {
        Arc::get_mut(&mut self.sh).expect("unshared").opts.translation = t;
        self
    }
    pub fn with_below(mut self, b: BelowMenu) -> Self {
        Arc::get_mut(&mut self.sh).expect("unshared").opts.below = b;
        self
    }
    /// All translation options at once (`TrackOpts::LEGACY` reproduces the pre-fix policy).
    pub fn with_opts(mut self, o: TrackOpts) -> Self {
        Arc::get_mut(&mut self.sh).expect("unshared").opts = o;
        self
    }
    pub fn opts(&self) -> TrackOpts {
        self.sh.opts
    }
    /// One-line JSON of the decision diagnostics (on-menu vs after-off-menu histories).
    pub fn diag_json(&self) -> String {
        let one = |d: &Diag| {
            let n = d.lookups.load(Relaxed).max(1) as f64;
            let m = |x: &AtomicU64| x.load(Relaxed) as f64 / 1e6 / n;
            format!(
                "{{\"decisions\":{},\"miss\":{:.4},\"desync\":{:.4},\"why\":[{},{},{},{},{}],\"p_fold\":{:.4},\"p_call\":{:.4},\"p_raise\":{:.4}}}",
                d.lookups.load(Relaxed),
                d.misses.load(Relaxed) as f64 / n,
                d.desync.load(Relaxed) as f64 / n,
                d.why[1].load(Relaxed),
                d.why[2].load(Relaxed),
                d.why[3].load(Relaxed),
                d.why[4].load(Relaxed),
                d.why[5].load(Relaxed),
                m(&d.fold),
                m(&d.call),
                m(&d.raise)
            )
        };
        format!("{{\"on_menu\":{},\"after_off_menu\":{}}}", one(&self.sh.diag[0]), one(&self.sh.diag[1]))
    }
    pub fn with_avg_preflop(mut self, on: bool) -> Self {
        Arc::get_mut(&mut self.sh).expect("unshared").avg_preflop = on;
        self
    }
    pub fn store(&self) -> &Store {
        &self.sh.store
    }
    /// Fraction of decisions whose infoset was missing from the store (fallback: check/call).
    pub fn miss_rate(&self) -> f64 {
        self.sh.misses.load(Relaxed) as f64 / self.sh.lookups.load(Relaxed).max(1) as f64
    }
    pub fn from_file(
        path: &std::path::Path,
        bucketer: &str,
        sizing: &str,
        capacity: usize,
    ) -> Result<Self, String> {
        let store = blueprint::io::load(path, capacity, 0).map_err(|e| e.to_string())?;
        let b = bucketer_by_name(bucketer).ok_or(format!("unknown bucketer {bucketer}"))?;
        let s = sizing_by_name(sizing).ok_or(format!("unknown sizing {sizing}"))?;
        Ok(Self::new(
            &format!("bp:{}", path.display()),
            Arc::new(store),
            b,
            s,
        ))
    }
}

impl PolicyFactory for BlueprintPolicy {
    fn name(&self) -> String {
        self.name.clone()
    }
    fn report(&self) -> Option<String> {
        Some(self.diag_json())
    }
    fn new_player(&self, _seat: usize, _cfg: &GameConfig, seed: u64) -> Box<dyn Player> {
        Box::new(BpPlayer {
            sh: self.sh.clone(),
            seat: _seat,
            track: Shadow::new(&State::new(_cfg), self.sh.opts),
            rng: Rng::new(seed),
        })
    }
}

#[derive(Clone)]
struct BpPlayer {
    sh: Arc<Shared>,
    seat: usize,
    track: Shadow,
    rng: Rng,
}

impl Player for BpPlayer {
    fn observe(&mut self, pre: &State, seat: usize, a: Action) {
        self.track
            .observe(pre, &self.sh.sizing, a, seat == self.seat, &mut self.rng);
    }

    fn dist(&mut self, v: &View, hole: [Card; 2]) -> Dist {
        let sh = &self.sh;
        let diag = &sh.diag[usize::from(self.track.off_tree())];
        sh.lookups.fetch_add(1, Relaxed);
        diag.lookups.fetch_add(1, Relaxed);
        let passive = |st: &State| {
            if st.legal().can_check {
                Action::Check
            } else {
                Action::Call
            }
        };
        let mut d = Dist::new();
        let why = self.track.sync_reason(v.state);
        if why != 0 {
            sh.misses.fetch_add(1, Relaxed);
            diag.misses.fetch_add(1, Relaxed);
            diag.desync.fetch_add(1, Relaxed);
            if diag.why[why as usize].fetch_add(1, Relaxed) < 6 && std::env::var_os("PLURIBUS_DEBUG_DESYNC").is_some() {
                eprintln!("DESYNC why={why}\n real  {:?}\n shadow {:?}\n hist {:?}", v.state, self.track, v.history);
            }
            if sh.opts.keep_sync {
                d.push(fallback_action(v.state, v.board, hole), 1.0);
            } else {
                d.push(passive(v.state), 1.0);
            }
            return d;
        }
        let mut list = ActionList::new();
        self.track.actions(v.state, &sh.sizing, &mut list);
        let n = list.len();
        let street = v.state.street();
        let bucket = sh.bucketer.bucket(street, hole, v.board);
        let key = self.track.key();
        let mut p = [0f32; nlhe::abstraction::MAX_ACTIONS];
        let found = if let Some(src) = &sh.source {
            src.probs(key, bucket, &mut p[..n])
        } else if let Some(slot) = sh
            .store
            .lookup(key, bucket)
            .filter(|q| q.n_actions as usize == n)
        {
            if sh.avg_preflop && street == Street::Preflop {
                sh.store.average(slot, &mut p[..n]);
            } else {
                sh.store.strategy(slot, &mut p[..n]);
            }
            true
        } else {
            false
        };
        if !found {
            sh.misses.fetch_add(1, Relaxed);
            diag.misses.fetch_add(1, Relaxed);
            if sh.opts.keep_sync {
                d.push(fallback_action(v.state, v.board, hole), 1.0);
            } else {
                d.push(passive(v.state), 1.0);
            }
            return d;
        }
        let tot: f32 = p[..n].iter().sum::<f32>().max(1e-12);
        for i in 0..n {
            let w = (p[i] / tot * 1e6) as u64;
            match list.get(i) {
                Action::Fold => diag.fold.fetch_add(w, Relaxed),
                Action::Check | Action::Call => diag.call.fetch_add(w, Relaxed),
                Action::RaiseTo(_) => diag.raise.fetch_add(w, Relaxed),
            };
            d.push(self.track.real_action(v.state, &list, i), p[i]);
        }
        d
    }

    fn exposes_probs(&self) -> bool {
        self.sh.opts.translation == Translation::Deterministic
    }
    fn fork(&self) -> Option<Box<dyn Player>> {
        Some(Box::new(self.clone()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bias {
    Fold,
    Call,
    Raise,
}

/// Pluribus continuation strategy: multiply one action class by 5 and renormalise.
pub struct Biased {
    pub inner: Arc<dyn PolicyFactory>,
    pub bias: Bias,
    pub factor: f32,
}

impl Biased {
    pub fn new(inner: Arc<dyn PolicyFactory>, bias: Bias) -> Self {
        Biased {
            inner,
            bias,
            factor: 5.0,
        }
    }
}

impl PolicyFactory for Biased {
    fn name(&self) -> String {
        format!(
            "{}+{}x{}",
            self.inner.name(),
            format!("{:?}", self.bias).to_lowercase(),
            self.factor
        )
    }
    fn report(&self) -> Option<String> {
        self.inner.report()
    }
    fn new_player(&self, seat: usize, cfg: &GameConfig, seed: u64) -> Box<dyn Player> {
        Box::new(BiasedPlayer {
            inner: self.inner.new_player(seat, cfg, seed),
            bias: self.bias,
            factor: self.factor,
        })
    }
}

struct BiasedPlayer {
    inner: Box<dyn Player>,
    bias: Bias,
    factor: f32,
}

impl Player for BiasedPlayer {
    fn observe(&mut self, pre: &State, seat: usize, a: Action) {
        self.inner.observe(pre, seat, a)
    }
    fn observe_board(&mut self, b: &[Card]) {
        self.inner.observe_board(b)
    }
    fn dist(&mut self, v: &View, hole: [Card; 2]) -> Dist {
        let d = self.inner.dist(v, hole);
        let mut out = Dist::new();
        for i in 0..d.len() {
            let a = d.action(i);
            let hit = match (self.bias, a) {
                (Bias::Fold, Action::Fold)
                | (Bias::Call, Action::Call | Action::Check)
                | (Bias::Raise, Action::RaiseTo(_)) => true,
                _ => false,
            };
            out.push(a, d.prob(i) as f32 * if hit { self.factor } else { 1.0 });
        }
        out
    }
    fn exposes_probs(&self) -> bool {
        self.inner.exposes_probs()
    }
    fn fork(&self) -> Option<Box<dyn Player>> {
        Some(Box::new(BiasedPlayer {
            inner: self.inner.fork()?,
            bias: self.bias,
            factor: self.factor,
        }))
    }
}

/// Train a small blueprint (toy schedule) for tests and demos.
pub fn train_toy(players: usize, iters: u64, seed: u64) -> BlueprintPolicy {
    use blueprint::trainer::{Schedule, Trainer};
    let game = GameConfig::new(players);
    let sched = Schedule {
        discount_interval: 1000,
        lcfr_until: iters / 3,
        prune_after: iters / 6,
        prune_prob: 0.95,
        prune_threshold: -3_000_000,
        strategy_interval: 10,
        avg_after: 0,
    };
    let tr = Trainer::new(
        game,
        BetSizing::tiny(),
        sched,
        &CoarseBucketer,
        1 << 25,
        seed,
    );
    tr.run(0, iters, |_, _, _| {});
    BlueprintPolicy::new(
        "toy-bp",
        tr.store,
        Arc::new(CoarseBucketer),
        BetSizing::tiny(),
    )
}

/// Equity of `hole` against `opps` random hands with the board completed at random (Monte Carlo,
/// seeded by the cards so the same spot always gets the same number).
fn mc_equity(hole: [Card; 2], board: &[Card], opps: usize, samples: u32) -> f64 {
    let dead = cards::mask_of(&hole) | cards::mask_of(board);
    let deck: Vec<Card> = cards::cards_in(cards::FULL_DECK & !dead).collect();
    let mut rng = Rng::new(dead ^ 0x9E37_79B9_7F4A_7C15);
    let need_board = 5 - board.len();
    let need = need_board + 2 * opps;
    let mut buf = deck.clone();
    let mut share = 0.0;
    for _ in 0..samples {
        for k in 0..need {
            let j = k + rng.below((buf.len() - k) as u32) as usize;
            buf.swap(k, j);
        }
        let mut b = cards::mask_of(board);
        for &c in &buf[..need_board] {
            b |= cards::bit(c);
        }
        let me = cards::eval::evaluate(b | cards::mask_of(&hole));
        let (mut best, mut ties) = (true, 1u32);
        for o in 0..opps {
            let h = &buf[need_board + 2 * o..need_board + 2 * o + 2];
            let r = cards::eval::evaluate(b | cards::mask_of(h));
            if r > me {
                best = false;
                break;
            }
            if r == me {
                ties += 1;
            }
        }
        if best {
            share += 1.0 / ties as f64;
        }
    }
    share / samples as f64
}

/// Play when the blueprint cannot be queried (the shadow lost the real game, or the infoset is
/// missing): by hand strength against the players still in, never a blind check/call. Facing a
/// bet it continues only with equity clearly above the price (betting ranges are stronger than
/// random hands); strong hands bet or raise about pot.
pub fn fallback_action(st: &State, board: &[Card], hole: [Card; 2]) -> Action {
    let me = st.to_act().unwrap_or(0);
    let opps = (0..st.num_players())
        .filter(|&s| s != me && !st.is_folded(s))
        .count()
        .max(1);
    let e = mc_equity(hole, board, opps, 400);
    let l = st.legal();
    let pot = st.pot() as f64;
    let raise = |frac: f64| {
        let to = st.cur_bet() + ((pot + l.call_amount as f64) * frac).round() as i32;
        Action::RaiseTo(to.clamp(l.min_raise_to, l.max_raise_to))
    };
    if l.can_check {
        return if l.can_raise && e > 0.70 { raise(0.75) } else { Action::Check };
    }
    let c = l.call_amount as f64;
    let price = c / (pot + c);
    if l.can_raise && e > 0.85 {
        raise(1.0)
    } else if e > price + 0.12 {
        Action::Call
    } else {
        Action::Fold
    }
}
