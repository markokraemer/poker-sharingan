//! Time-boxed search policy for live play.
//!
//! [`GuardedSearch`] wraps a [`SearchPolicy`] so that a decision can never take longer than the
//! budget and a failing search can never take the bot down with it:
//!
//! * Two layers. (1) the solver's cooperative cap (`SearchParams::max_millis`, set by the caller
//!   to a fraction of the budget) makes a slow search return a truncated-but-valid solution;
//!   (2) the hard guarantee: every search runs on a worker thread against a **clone** of the
//!   player and the decision thread waits at most `budget` (`recv_timeout`). On timeout (or a
//!   worker panic that unwinds) the decision uses the blueprint action and the trace gets a
//!   `fallback_reason`.
//! * State safety. The worker owns a clone; the real player is only replaced by the clone when
//!   the worker finished in time. On timeout the real player is `detach_search`ed: fresh
//!   solution cell (the abandoned worker holds the old cell's lock) and blueprint play until the
//!   next street (`observe_board` re-arms search). Its beliefs/history are untouched, because
//!   `observe` calls only ever hit the real player.
//! * At most one abandoned search at a time: while one is still running (`inflight > 0`) new
//!   searches are skipped (blueprint) so abandoned work cannot pile up on the thread pool.
//!
//! Caveat: a panic can only be contained if the binary is built with `panic = "unwind"`;
//! the workspace release profile uses `panic = "abort"`, where a search panic aborts the process.

use bench::policy::{Dist, Event, Player, PolicyFactory, View};
use cards::Card;
use nlhe::{Action, GameConfig, State};
use search::policy::{SearchPlayer, SearchPolicy};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct GuardStats {
    searched: AtomicU64,
    timeouts: AtomicU64,
    panics: AtomicU64,
    skipped_busy: AtomicU64,
    /// Wall time (us) of each decision that ran a search (success or not).
    micros: Mutex<Vec<u64>>,
}

pub struct GuardedSearch {
    name: String,
    policy: Arc<SearchPolicy>,
    budget: Duration,
    inflight: Arc<AtomicUsize>,
    stats: Arc<GuardStats>,
}

impl GuardedSearch {
    pub fn new(name: &str, policy: Arc<SearchPolicy>, budget: Duration) -> Self {
        GuardedSearch { name: name.into(), policy, budget, inflight: Arc::new(AtomicUsize::new(0)), stats: Arc::default() }
    }
    pub fn budget(&self) -> Duration {
        self.budget
    }
    pub fn policy(&self) -> &Arc<SearchPolicy> {
        &self.policy
    }
    pub fn timeouts(&self) -> u64 {
        self.stats.timeouts.load(Relaxed)
    }
    pub fn panics(&self) -> u64 {
        self.stats.panics.load(Relaxed)
    }
    pub fn searched(&self) -> u64 {
        self.stats.searched.load(Relaxed)
    }
    /// (mean, p50, p95, max) in ms over decisions that ran a search.
    pub fn latency_ms(&self) -> Option<(f64, f64, f64, f64)> {
        let mut v = self.stats.micros.lock().unwrap().clone();
        if v.is_empty() {
            return None;
        }
        v.sort_unstable();
        let q = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize] as f64 / 1e3;
        Some((v.iter().sum::<u64>() as f64 / v.len() as f64 / 1e3, q(0.5), q(0.95), *v.last().unwrap() as f64 / 1e3))
    }
}

impl PolicyFactory for GuardedSearch {
    fn name(&self) -> String {
        self.name.clone()
    }
    fn new_player(&self, seat: usize, cfg: &GameConfig, seed: u64) -> Box<dyn Player> {
        Box::new(GuardedPlayer {
            p: self.policy.player(seat, cfg, seed),
            budget: self.budget,
            inflight: self.inflight.clone(),
            stats: self.stats.clone(),
            note: None,
        })
    }
    fn report(&self) -> Option<String> {
        let s = &self.stats;
        let lat = self.latency_ms().map(|(m, p50, p95, mx)| format!(r#","mean_ms":{m:.1},"p50_ms":{p50:.1},"p95_ms":{p95:.1},"max_ms":{mx:.1}"#)).unwrap_or_default();
        Some(format!(
            r#"{{"budget_ms":{},"searched":{},"timeouts":{},"panics":{},"skipped_busy":{}{lat}}}"#,
            self.budget.as_millis(),
            s.searched.load(Relaxed),
            s.timeouts.load(Relaxed),
            s.panics.load(Relaxed),
            s.skipped_busy.load(Relaxed)
        ))
    }
}

pub struct GuardedPlayer {
    p: SearchPlayer,
    budget: Duration,
    inflight: Arc<AtomicUsize>,
    stats: Arc<GuardStats>,
    note: Option<String>,
}

struct InflightGuard(Arc<AtomicUsize>);
impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Relaxed);
    }
}

impl GuardedPlayer {
    fn fallback(&mut self, v: &View, hole: [Card; 2], why: String) -> Dist {
        self.p.detach_search();
        self.note = Some(why);
        self.p.blueprint_dist(v, hole)
    }
}

impl Player for GuardedPlayer {
    fn observe(&mut self, pre: &State, seat: usize, a: Action) {
        self.p.observe(pre, seat, a);
    }
    fn observe_board(&mut self, board: &[Card]) {
        self.p.observe_board(board);
    }
    fn dist(&mut self, v: &View, hole: [Card; 2]) -> Dist {
        self.note = None;
        if !self.p.needs_search(v) {
            // Cheap path (blueprint, or an existing solution): no search will run.
            return self.p.dist(v, hole);
        }
        let t0 = Instant::now();
        if self.inflight.load(Relaxed) > 0 {
            self.stats.skipped_busy.fetch_add(1, Relaxed);
            return self.fallback(v, hole, "prior_search_still_running: blueprint".into());
        }
        let mut worker = self.p.clone();
        let (cfg, state, seat) = (*v.cfg, *v.state, v.seat);
        let (board, history): (Vec<Card>, Vec<Event>) = (v.board.to_vec(), v.history.to_vec());
        let (tx, rx) = mpsc::channel();
        self.inflight.fetch_add(1, Relaxed);
        let guard = InflightGuard(self.inflight.clone());
        let spawned = std::thread::Builder::new().name("search-decision".into()).spawn(move || {
            let _guard = guard;
            let view = View { cfg: &cfg, state: &state, board: &board, history: &history, seat };
            let d = worker.dist(&view, hole);
            let _ = tx.send((worker, d));
        });
        if let Err(e) = spawned {
            // `guard` was moved into the closure that failed to start and has been dropped with it.
            return self.fallback(v, hole, format!("spawn_failed: {e}"));
        }
        let res = rx.recv_timeout(self.budget);
        self.stats.micros.lock().unwrap().push(t0.elapsed().as_micros() as u64);
        match res {
            Ok((worker, d)) => {
                self.stats.searched.fetch_add(1, Relaxed);
                self.p = worker;
                d
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.stats.timeouts.fetch_add(1, Relaxed);
                self.fallback(v, hole, format!("search_timeout>{}ms: blueprint", self.budget.as_millis()))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.stats.panics.fetch_add(1, Relaxed);
                self.fallback(v, hole, "search_panic: blueprint".into())
            }
        }
    }
    fn exposes_probs(&self) -> bool {
        true
    }
    fn take_note(&mut self) -> Option<String> {
        self.note.take()
    }
}
