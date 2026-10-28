//! `SearchPolicy`: the Pluribus real-time search agent as a `bench` [`PolicyFactory`],
//! implementing supplementary Algorithm 2 (nested search).
//!
//! Per hand each [`Player`] tracks, from the stream of `observe` / `observe_board` calls,
//!
//! * the **blueprint history** (real raises translated to the nearest blueprint size),
//! * the **beliefs** over every seat's 1326 hole pairs at the root of the current betting
//!   round, Bayes-updated at each round end with the *average* strategy of the round's search
//!   (blueprint where the round was not searched),
//! * the **subgame**: root = start of the current betting round, `extras` = off-tree opponent
//!   raises added as legal actions, `frozen` = our own actions this round (kept only for our
//!   actual hand).
//!
//! Algorithm 2 mapping: `OpponentTurn` = `observe` of another seat (an off-tree raise adds the
//! action and marks the solution stale, re-searched from the same root at the next decision);
//! `OurTurn` = `dist` (sample from the final iterate, action frozen in `observe`);
//! `CheckNewRound` = `observe_board` (new root, beliefs updated, search at the next decision).
//! Searching is lazy: it runs the first time `dist` is called with the round's state because
//! the actual hole cards are only revealed to the player through `dist`.
//!
//! `dist(view, hole)` answers for *any* hole cards from the same solution (the range
//! strategy), so AIVAT and LBR can query it; only our actual hand has its earlier actions of
//! the round frozen. `fork()` yields a copy that shares the round's solution, may run the
//! round's first search if none exists, and never re-searches (an off-tree action in a fork
//! degrades it to blueprint play for the rest of the hand).

use crate::beliefs::{holes, Beliefs};
use crate::bp::Blueprint;
use crate::solver::{solve, SearchParams, Solution, Subgame};
use crate::tree::{search_actions, step_path, Extras};
use bench::mix;
use bench::policy::{Dist, Player, PolicyFactory, View};
use cards::{hole_index, mask_of, Card};
use nlhe::abstraction::{ActionList, Shadow, MAX_ACTIONS};
use nlhe::{Action, GameConfig, State, Street};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Default)]
struct StatsInner {
    searches: AtomicU64,
    micros: AtomicU64,
    max_micros: AtomicU64,
    iters: AtomicU64,
    infosets: AtomicU64,
    resolves: AtomicU64,
    preflop: AtomicU64,
    max_vec_bytes: AtomicU64,
    max_infosets: AtomicU64,
}

#[derive(Clone, Copy, Debug)]
pub struct Stats {
    pub searches: u64,
    pub mean_ms: f64,
    pub max_ms: f64,
    pub mean_iters: f64,
    pub mean_infosets: f64,
    /// Searches caused by an off-tree action (re-search from the same root).
    pub resolves: u64,
    /// Searches on the first betting round.
    pub preflop: u64,
    /// Largest vector-solver node storage of any search, MB.
    pub max_vec_mb: f64,
    pub max_infosets: u64,
}

struct Shared {
    bp: Blueprint,
    params: SearchParams,
    stats: StatsInner,
}

pub struct SearchPolicy {
    name: String,
    sh: Arc<Shared>,
}

impl SearchPolicy {
    pub fn new(name: &str, bp: Blueprint, params: SearchParams) -> Self {
        SearchPolicy {
            name: name.to_string(),
            sh: Arc::new(Shared { bp, params, stats: StatsInner::default() }),
        }
    }
    pub fn params(&self) -> &SearchParams {
        &self.sh.params
    }
    pub fn blueprint(&self) -> &Blueprint {
        &self.sh.bp
    }
    pub fn stats(&self) -> Stats {
        let s = &self.sh.stats;
        let n = s.searches.load(Relaxed).max(1) as f64;
        Stats {
            searches: s.searches.load(Relaxed),
            mean_ms: s.micros.load(Relaxed) as f64 / n / 1e3,
            max_ms: s.max_micros.load(Relaxed) as f64 / 1e3,
            mean_iters: s.iters.load(Relaxed) as f64 / n,
            mean_infosets: s.infosets.load(Relaxed) as f64 / n,
            resolves: s.resolves.load(Relaxed),
            preflop: s.preflop.load(Relaxed),
            max_vec_mb: s.max_vec_bytes.load(Relaxed) as f64 / 1e6,
            max_infosets: s.max_infosets.load(Relaxed),
        }
    }
    /// A fresh player (also usable directly in tests).
    pub fn player(&self, seat: usize, cfg: &GameConfig, seed: u64) -> SearchPlayer {
        SearchPlayer::new(self.sh.clone(), seat, cfg, seed)
    }
}

impl PolicyFactory for SearchPolicy {
    fn name(&self) -> String {
        self.name.clone()
    }
    fn new_player(&self, seat: usize, cfg: &GameConfig, seed: u64) -> Box<dyn Player> {
        Box::new(self.player(seat, cfg, seed))
    }
}

#[derive(Clone)]
struct Ev {
    seat: usize,
    pre: State,
    action: Action,
    path: u64,
    /// Blueprint shadow game before `action`.
    sh: Shadow,
}

type Cell = Arc<Mutex<Option<Arc<Solution>>>>;

#[derive(Clone)]
pub struct SearchPlayer {
    sh: Arc<Shared>,
    cfg: GameConfig,
    seat: usize,
    seed: u64,
    state: State,
    board: Vec<Card>,
    hole: Option<[Card; 2]>,
    out: bool,
    bp_sh: Shadow,
    root: State,
    root_sh: Shadow,
    root_beliefs: Arc<Beliefs>,
    events: Vec<Ev>,
    path: u64,
    extras: Extras,
    frozen: HashMap<u64, Action>,
    search_active: bool,
    cell: Cell,
    can_resolve: bool,
    degraded: bool,
    n_searches: u64,
}

fn n_extras(e: &Extras) -> usize {
    e.values().map(|v| v.len()).sum()
}

impl SearchPlayer {
    fn new(sh: Arc<Shared>, seat: usize, cfg: &GameConfig, seed: u64) -> Self {
        let state = State::new(cfg);
        let sh_bp_new = sh.bp.shadow(&state);
        SearchPlayer {
            sh,
            cfg: *cfg,
            seat,
            seed,
            state,
            board: vec![],
            hole: None,
            out: false,
            bp_sh: sh_bp_new,
            root: state,
            root_sh: sh_bp_new,
            root_beliefs: Arc::new(Beliefs::uniform(cfg.players)),
            events: vec![],
            path: 0,
            extras: Extras::new(),
            frozen: HashMap::new(),
            search_active: false,
            cell: Arc::new(Mutex::new(None)),
            can_resolve: true,
            degraded: false,
            n_searches: 0,
        }
    }

    /// The current round's solution, if any.
    pub fn solution(&self) -> Option<Arc<Solution>> {
        self.cell.lock().unwrap().clone()
    }
    /// Off-tree raise amounts added to the current round's subgame (per public node).
    pub fn extras(&self) -> &Extras {
        &self.extras
    }
    pub fn search_active(&self) -> bool {
        self.search_active
    }
    pub fn beliefs(&self) -> &Beliefs {
        &self.root_beliefs
    }
    pub fn searches_run(&self) -> u64 {
        self.n_searches
    }
    /// Root state and blueprint history of the current betting round, and the board so far.
    pub fn round_root(&self) -> (State, Shadow, &[Card]) {
        (self.root, self.root_sh, &self.board)
    }

    /// Would `dist` at `v` run a (blocking, possibly long) search right now?
    pub fn needs_search(&self, v: &View) -> bool {
        self.need_search() && !v.state.is_terminal()
    }

    /// Plain blueprint answer (translated history), ignoring any search state.
    pub fn blueprint_dist(&self, v: &View, hole: [Card; 2]) -> Dist {
        self.sh.bp.dist(v.state, &self.bp_sh, hole, v.board)
    }

    /// Abandon the current round's search: this player plays the blueprint for the rest of the
    /// betting round (until the next `observe_board`) and no longer shares the old solution
    /// cell, so a search still running on a clone of this player (whose lock it holds) can
    /// neither block nor corrupt this one.
    pub fn detach_search(&mut self) {
        self.cell = Arc::new(Mutex::new(None));
        self.degraded = true;
    }

    fn need_search(&self) -> bool {
        if !self.search_active || self.degraded {
            return false;
        }
        match &*self.cell.lock().unwrap() {
            None => true,
            Some(s) => self.can_resolve && n_extras(&s.extras) != n_extras(&self.extras),
        }
    }

    fn run_search(&mut self) {
        let sh = self.sh.clone();
        let mut g = self.cell.lock().unwrap();
        let resolve = g.is_some();
        // A fork may find the first solution already done by a sibling.
        if let Some(s) = &*g {
            if !self.can_resolve || n_extras(&s.extras) == n_extras(&self.extras) {
                return;
            }
        }
        let t0 = Instant::now();
        let sg = Subgame {
            bp: &sh.bp,
            root: self.root,
            board: &self.board,
            beliefs: &self.root_beliefs,
            bp_hist: self.root_sh,
            extras: &self.extras,
            me: self.seat,
            my_hole: if self.can_resolve { self.hole } else { None },
            frozen: &self.frozen,
            params: &sh.params,
            seed: mix(self.seed, self.n_searches + 1),
        };
        let sol = solve(&sg);
        let us = t0.elapsed().as_micros() as u64;
        let st = &sh.stats;
        st.searches.fetch_add(1, Relaxed);
        st.micros.fetch_add(us, Relaxed);
        st.max_micros.fetch_max(us, Relaxed);
        st.iters.fetch_add(sol.iters, Relaxed);
        st.max_vec_bytes.fetch_max(sol.peak_bytes() as u64, Relaxed);
        st.infosets.fetch_add(sol.infosets() as u64, Relaxed);
        st.max_infosets.fetch_max(sol.infosets() as u64, Relaxed);
        if resolve {
            st.resolves.fetch_add(1, Relaxed);
        }
        if self.root.street() == Street::Preflop {
            st.preflop.fetch_add(1, Relaxed);
        }
        *g = Some(Arc::new(sol));
        drop(g);
        self.n_searches += 1;
    }

    /// Range strategy of the current solution at the current node, as real actions.
    fn search_dist(&self, sol: &Solution, st: &State, hole: [Card; 2]) -> Option<Dist> {
        if sol.root_street != st.street() {
            return None;
        }
        let mut list = ActionList::new();
        let ex: &[i32] = sol.extras.get(&self.path).map_or(&[], |v| v.as_slice());
        search_actions(st, &sol.sizing, ex, &mut list);
        let n = list.len();
        let mut p = [0f32; MAX_ACTIONS];
        if !sol.probs(self.path, hole_index(hole[0], hole[1]), n, self.sh.params.play_final, &mut p) {
            return None;
        }
        let mut d = Dist::new();
        for i in 0..n {
            d.push(list.get(i), p[i]);
        }
        Some(d)
    }

    fn consider_off_tree(&mut self, pre: &State, a: Action, path: u64) {
        let Action::RaiseTo(t) = a else { return };
        let prm = &self.sh.params;
        if self.search_active {
            let mut list = ActionList::new();
            let ex: &[i32] = self.extras.get(&path).map_or(&[], |v| v.as_slice());
            search_actions(pre, &prm.sizing, ex, &mut list);
            if !list.as_slice().contains(&a) {
                if self.can_resolve {
                    self.extras.entry(path).or_default().push(t);
                } else {
                    self.degraded = true;
                }
            }
        } else if self.can_resolve && prm.preflop_search && pre.street() == Street::Preflop {
            // Off-tree by more than one big blind-sized margin ($100 in the paper) vs every
            // blueprint raise size, with at most 4 players left.
            let thr = prm.preflop_trigger_chips.unwrap_or(self.cfg.big_blind);
            // Blueprint raise sizes at the shadow node, expressed as real chip amounts.
            let mut list = ActionList::new();
            let bp = &self.sh.bp;
            let sizes: Vec<i32> = if bp.node_list(&self.bp_sh, pre, &mut list) {
                (0..list.len())
                    .filter_map(|i| match self.bp_sh.real_action(pre, &list, i) {
                        Action::RaiseTo(r) => Some(r),
                        _ => None,
                    })
                    .collect()
            } else {
                vec![]
            };
            let far = sizes.iter().all(|&r| (r - t).abs() > thr);
            let mut after = *pre;
            after.apply(a);
            if far && after.live_count() as usize <= prm.preflop_max_players {
                self.search_active = true;
                self.extras.entry(path).or_default().push(t);
            }
        }
    }

    /// Bayes-update the round-root beliefs with every action of the finished round.
    fn finish_round(&mut self, new_board: &[Card]) {
        let mut b = (*self.root_beliefs).clone();
        let prm = &self.sh.params;
        let sol = if self.search_active { self.solution() } else { None };
        for ev in &self.events {
            // `blueprint_beliefs`: Bayes with the blueprint instead of the search average, except
            // (`own_search_beliefs`) for our own seat, whose real strategy IS the search's.
            let off_tree = prm.offtree_search_beliefs
                && matches!(ev.action, Action::RaiseTo(t) if sol.as_deref().is_some_and(|s| s.extras.get(&ev.path).is_some_and(|v| v.contains(&t))));
            let use_search = !prm.blueprint_beliefs || (prm.own_search_beliefs && ev.seat == self.seat) || off_tree;
            self.update_event(&mut b, ev, if use_search { sol.as_deref() } else { None });
        }
        b.remove_cards(mask_of(new_board));
        if let Some(h) = self.hole {
            b.floor(self.seat, hole_index(h[0], h[1]), 1e-6);
        }
        self.root_beliefs = Arc::new(b);
    }

    fn update_event(&self, b: &mut Beliefs, ev: &Ev, sol: Option<&Solution>) {
        let bp = &self.sh.bp;
        let street = ev.pre.street();
        let blen = street.board_len().min(self.board.len());
        let board = &self.board[..blen];
        let hl = holes();
        let mut list_b = ActionList::new();
        let synced = bp.node_list(&ev.sh, &ev.pre, &mut list_b);
        let nb = list_b.len().max(1);
        let ab = if synced { bp.translate_in(&ev.sh, &ev.pre, &list_b, ev.action, ev.seat == self.seat) } else { 0 };
        // Opponent tremble (our own seat's likelihood is exact: we know our strategy).
        let eps = if ev.seat == self.seat { 0.0 } else { self.sh.params.belief_tremble.clamp(0.0, 1.0) };
        let mut buf = [0f32; MAX_ACTIONS];
        let bp_lik = |i: usize, buf: &mut [f32; MAX_ACTIONS]| -> f32 {
            let bucket = bp.bucket(street, hl[i], board);
            if bp.sigma(&ev.sh, &ev.pre, &list_b, bucket, buf) {
                buf[ab]
            } else {
                1.0 / nb as f32
            }
        };
        if let Some(sol) = sol.filter(|s| s.root_street == street) {
            let mut list = ActionList::new();
            let ex: &[i32] = sol.extras.get(&ev.path).map_or(&[], |v| v.as_slice());
            search_actions(&ev.pre, &sol.sizing, ex, &mut list);
            if let Some(ai) = list.as_slice().iter().position(|x| *x == ev.action) {
                let n = list.len();
                let pooled = sol.opp_pool && ev.seat != sol.me;
                b.bayes(ev.seat, |i| {
                    let hit = if pooled {
                        sol.probs_opp(ev.path, bp.bucket(street, hl[i], board), n, false, &mut buf)
                    } else {
                        sol.probs(ev.path, i, n, false, &mut buf)
                    };
                    let l = if hit { buf[ai] } else { bp_lik(i, &mut buf) };
                    (1.0 - eps) * l + eps / n as f32
                });
                return;
            }
        }
        // Shadow lost the game (unrepresentable action): no usable blueprint likelihood.
        if synced {
            b.bayes(ev.seat, |i| (1.0 - eps) * bp_lik(i, &mut buf) + eps / nb as f32);
        }
    }
}

impl Player for SearchPlayer {
    fn observe(&mut self, pre: &State, seat: usize, a: Action) {
        let path = self.path;
        self.events.push(Ev { seat, pre: *pre, action: a, path, sh: self.bp_sh });
        if seat == self.seat {
            self.frozen.insert(path, a);
            if a == Action::Fold {
                self.out = true;
            }
        } else if !self.out {
            self.consider_off_tree(pre, a, path);
        }
        // After the off-tree check (which reads the pre-action shadow node).
        self.sh.bp.observe(&mut self.bp_sh, pre, a, seat == self.seat);
        self.path = step_path(path, a);
        self.state = *pre;
        self.state.apply(a);
    }

    fn observe_board(&mut self, board: &[Card]) {
        if board.len() <= self.board.len() {
            return;
        }
        if !self.out {
            self.finish_round(board);
        }
        self.board = board.to_vec();
        self.root = self.state;
        self.root_sh = self.bp_sh;
        self.events.clear();
        self.path = 0;
        self.extras.clear();
        self.frozen.clear();
        self.cell = Arc::new(Mutex::new(None));
        self.degraded = false;
        let prm = &self.sh.params;
        let live = self.state.live_count() as usize;
        let street_ok = (board.len() == 5 && prm.search_river && live <= prm.max_players[2])
            || (board.len() == 4 && prm.search_turn && live <= prm.max_players[1])
            || (board.len() == 3 && prm.search_flop && live <= prm.max_players[0] && live >= prm.min_flop_players);
        self.search_active = prm.postflop_search && !self.out && street_ok;
    }

    fn dist(&mut self, v: &View, hole: [Card; 2]) -> Dist {
        if self.can_resolve && self.hole.is_none() {
            self.hole = Some(hole);
        }
        if self.need_search() && !v.state.is_terminal() {
            self.run_search();
        }
        if self.search_active && !self.degraded {
            if let Some(sol) = self.solution() {
                if let Some(d) = self.search_dist(&sol, v.state, hole) {
                    return d;
                }
            }
        }
        self.sh.bp.dist(v.state, &self.bp_sh, hole, v.board)
    }

    fn exposes_probs(&self) -> bool {
        true
    }

    fn fork(&self) -> Option<Box<dyn Player>> {
        let mut f = self.clone();
        f.can_resolve = false;
        Some(Box::new(f))
    }
}
