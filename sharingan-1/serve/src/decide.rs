//! The bot: one [`Core`] per process, shared by every request.
//!
//! * The compact blueprint is mmapped once at startup (`Core::new`) and shared (lock-free reads).
//! * Each request builds a fresh [`search::policy::SearchPlayer`] by replaying the history
//!   ([`crate::replay`]); the turn/river decision then runs a real-time search **on a worker thread
//!   under a hard budget** (same two layers as `arena/openpoker/src/guard.rs`): the solver's
//!   cooperative cap (70% of the budget) returns a truncated-but-valid solution; the hard
//!   `recv_timeout` guarantees an answer, falling back to the blueprint action.
//! * A [`Gate`] bounds concurrent searches (they share one rayon pool). A timed-out search keeps
//!   its slot until it really ends, so abandoned work can never pile up.

use crate::api::*;
use crate::config::Config;
use crate::error::ApiError;
use crate::replay::{street_name, Hand};
use bench::bp::{bucketer_by_name, sizing_by_name};
use bench::mix;
use bench::policy::{Dist, Player, View};
use blueprint::compact::Compact;
use cards::Card;
use nlhe::abstraction::{BetSizing, TrackOpts};
use nlhe::Street;
use search::bp::Blueprint;
use search::policy::{SearchPlayer, SearchPolicy};
use search::solver::SearchParams;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------------------- gate

/// Counting semaphore with a timed acquire (decisions run on blocking threads).
pub struct Gate {
    used: Mutex<usize>,
    cv: Condvar,
    cap: usize,
}

pub struct Permit(Arc<Gate>);

impl Drop for Permit {
    fn drop(&mut self) {
        *self.0.used.lock().unwrap() -= 1;
        self.0.cv.notify_one();
    }
}

impl Gate {
    pub fn new(cap: usize) -> Arc<Gate> {
        Arc::new(Gate { used: Mutex::new(0), cv: Condvar::new(), cap })
    }
    pub fn acquire(g: &Arc<Gate>, wait: Duration) -> Option<Permit> {
        let deadline = Instant::now() + wait;
        let mut used = g.used.lock().unwrap();
        loop {
            if *used < g.cap {
                *used += 1;
                return Some(Permit(g.clone()));
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            let (u, to) = g.cv.wait_timeout(used, left).unwrap();
            used = u;
            if to.timed_out() && *used >= g.cap {
                return None;
            }
        }
    }
    pub fn in_use(&self) -> usize {
        *self.used.lock().unwrap()
    }
    pub fn capacity(&self) -> usize {
        self.cap
    }
}

// -------------------------------------------------------------------------------- search params

/// Parse `k=v,...` overrides of `SearchParams` (same keys as `openpoker --set` / `vsbp --set`).
pub fn apply_sets(prm: &mut SearchParams, sets: &str) -> Result<(), String> {
    fn p<T: std::str::FromStr>(k: &str, v: &str) -> Result<T, String> {
        v.parse().map_err(|_| format!("--set {k}={v}: bad value"))
    }
    for kv in sets.split(',').filter(|x| !x.is_empty()) {
        let (k, v) = kv.split_once('=').unwrap_or((kv, "1"));
        let b = v != "0" && v != "false";
        match k {
            "iters" => prm.iters = p(k, v)?,
            "warm" => prm.warm_start = p(k, v)?,
            "focus" => prm.focus_own = p(k, v)?,
            "leaf" => prm.leaf = if v == "bp" { search::solver::LeafMode::Blueprint } else { search::solver::LeafMode::Cont4 },
            "final" => prm.play_final = b,
            "flop" => prm.search_flop = b,
            "turn" => prm.search_turn = b,
            "river" => prm.search_river = b,
            "pre" => prm.preflop_search = b,
            "post" => prm.postflop_search = b,
            "bpbeliefs" => prm.blueprint_beliefs = b,
            "offbel" => prm.offtree_search_beliefs = b,
            "ownbel" => prm.own_search_beliefs = b,
            "tremble" => prm.belief_tremble = p(k, v)?,
            "prune" => prm.prune_prob = p(k, v)?,
            "ms" => prm.max_millis = Some(p(k, v)?),
            "minflop" => prm.min_flop_players = p(k, v)?,
            "vecmin" => prm.vec_min_street = match v { "flop" => Street::Flop, "turn" => Street::Turn, _ => Street::River },
            "vector" => prm.vector = b,
            "sizing" | "later" => {
                let s: BetSizing = match v {
                    "small" => search::tree::small_sizing(),
                    o => sizing_by_name(o).ok_or(format!("--set {k}={o}: unknown sizing"))?,
                };
                if k == "sizing" {
                    prm.sizing = s;
                } else {
                    prm.later_sizing = s;
                }
            }
            _ => return Err(format!("unknown --set key {k}")),
        }
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------ core

#[derive(Default)]
struct Counters {
    requests: AtomicU64,
    searched: AtomicU64,
    timeouts: AtomicU64,
    panics: AtomicU64,
    busy: AtomicU64,
    fallbacks: AtomicU64,
    search_micros: AtomicU64,
    search_max_micros: AtomicU64,
}

/// Blueprint facts reported by `/v1/info`.
pub struct Meta {
    pub path: String,
    pub infosets: u64,
    pub file_bytes: u64,
    pub src_infosets: u64,
    pub used_snapshots: bool,
}

pub struct Core {
    pub cfg: Config,
    blueprint: Blueprint,
    prm: SearchParams,
    gate: Arc<Gate>,
    meta: Meta,
    started: Instant,
    c: Counters,
    seq: AtomicU64,
}

impl Core {
    /// Load the compact blueprint, the card abstraction and the search pool.
    pub fn new(cfg: &Config) -> Result<Core, String> {
        let buckets = if cfg.buckets == "coarse" || cfg.buckets.starts_with("tables:") { cfg.buckets.clone() } else { format!("tables:{}", cfg.buckets) };
        let bucketer = bucketer_by_name(&buckets).ok_or_else(|| format!("cannot load card abstraction {buckets}"))?;
        let sizing = sizing_by_name(&cfg.sizing).ok_or_else(|| format!("unknown sizing {}", cfg.sizing))?;
        let compact = Arc::new(Compact::open(&cfg.blueprint).map_err(|e| format!("blueprint {}: {e}", cfg.blueprint.display()))?);
        if cfg.warm {
            compact.warm();
        } else {
            compact.random_access();
        }
        let meta = Meta {
            path: cfg.blueprint.display().to_string(),
            infosets: compact.len() as u64,
            file_bytes: compact.file_bytes(),
            src_infosets: compact.info().src_infosets,
            used_snapshots: compact.info().used_snapshots,
        };
        let bp = Blueprint::from_source(compact, bucketer, sizing).with_opts(TrackOpts::default());
        Self::with_blueprint(cfg, bp, meta)
    }

    /// Same over an already-built blueprint (tests).
    pub fn with_blueprint(cfg: &Config, bp: Blueprint, meta: Meta) -> Result<Core, String> {
        let mut prm = SearchParams::default();
        apply_sets(&mut prm, &cfg.sets)?;
        if let Some(dir) = &cfg.search_buckets {
            let sb = abstraction::search_buckets::SearchBuckets::new(Some(dir.into()), abstraction::search_buckets::BuildParams::default(), 64)
                .map_err(|e| format!("search buckets {dir}: {e}"))?;
            prm.search_buckets = Some(Arc::new(sb));
        }
        if !cfg.search {
            prm.postflop_search = false;
            prm.preflop_search = false;
        }
        prm.parallel = true;
        prm.pool = Some(Arc::new(
            rayon::ThreadPoolBuilder::new()
                .num_threads(cfg.search_threads.max(1))
                .thread_name(|i| format!("search-pool-{i}"))
                .build()
                .map_err(|e| format!("thread pool: {e}"))?,
        ));
        Ok(Core {
            cfg: cfg.clone(),
            blueprint: bp,
            prm,
            gate: Gate::new(cfg.search_concurrency.max(1)),
            meta,
            started: Instant::now(),
            c: Counters::default(),
            seq: AtomicU64::new(0),
        })
    }

    pub fn gate(&self) -> &Arc<Gate> {
        &self.gate
    }

    pub fn next_request_id(&self) -> String {
        format!("r-{:x}-{}", crate::log::now_ms(), self.seq.fetch_add(1, Relaxed))
    }

    /// `GET /v1/info`.
    pub fn info(&self) -> Value {
        let p = &self.prm;
        let n = self.c.searched.load(Relaxed).max(1) as f64;
        json!({
            "model_id": self.cfg.model_id,
            "name": "SHARINGAN-1",
            "version": env!("CARGO_PKG_VERSION"),
            "blueprint": {
                "path": self.meta.path, "infosets": self.meta.infosets, "file_bytes": self.meta.file_bytes,
                "source_infosets": self.meta.src_infosets, "snapshot_averaged": self.meta.used_snapshots,
                "format": "PLCP1 (compact, mmap)",
            },
            "sizing": self.cfg.sizing,
            "buckets": self.cfg.buckets,
            "players": "2-6 (trained for 6-max, 100 bb; 3-5 players embedded in the 6-max tree)",
            "search": {
                "enabled": self.cfg.search,
                "set": self.cfg.sets,
                "iters": p.iters,
                "flop": p.search_flop, "turn": p.search_turn, "river": p.search_river,
                "preflop_off_tree": p.preflop_search,
                "play_final": p.play_final,
                "leaf": format!("{:?}", p.leaf),
                "budget_ms": self.cfg.budget_ms,
                "cooperative_cap_ms": (self.cfg.budget_ms * 7 / 10).max(1),
                "threads": self.cfg.search_threads,
                "concurrency": self.gate.capacity(),
                "queue_ms": self.cfg.queue_ms,
            },
            "max_stack_bb": self.cfg.max_stack_bb,
            "limits": {"max_batch": self.cfg.max_batch, "rate_rps": self.cfg.rate_rps, "rate_burst": self.cfg.rate_burst, "max_inflight": self.cfg.max_inflight},
            "panic_strategy": if cfg!(panic = "unwind") { "unwind" } else { "abort" },
            "stats": {
                "uptime_s": self.started.elapsed().as_secs(),
                "requests": self.c.requests.load(Relaxed),
                "searched": self.c.searched.load(Relaxed),
                "search_timeouts": self.c.timeouts.load(Relaxed),
                "search_panics": self.c.panics.load(Relaxed),
                "search_busy": self.c.busy.load(Relaxed),
                "fallbacks": self.c.fallbacks.load(Relaxed),
                "search_mean_ms": self.c.search_micros.load(Relaxed) as f64 / n / 1e3,
                "search_max_ms": self.c.search_max_micros.load(Relaxed) as f64 / 1e3,
                "searches_in_flight": self.gate.in_use(),
            },
        })
    }

    pub fn health(&self) -> Value {
        json!({"status": "ok", "model_id": self.cfg.model_id, "uptime_s": self.started.elapsed().as_secs(), "searches_in_flight": self.gate.in_use()})
    }

    /// Answer one request. Blocking: call from a blocking thread.
    pub fn decide(&self, req: &ActRequest, request_id: String) -> Result<ActResponse, ApiError> {
        let t0 = Instant::now();
        self.c.requests.fetch_add(1, Relaxed);
        let opts = &req.options;
        if opts.time_budget_ms == Some(0) {
            return Err(ApiError::invalid("bad_budget", "options.time_budget_ms must be > 0"));
        }
        let budget_ms = opts.time_budget_ms.map_or(self.cfg.budget_ms, |b| b.min(self.cfg.budget_ms));
        let seed = opts.seed.unwrap_or_else(|| mix(crate::log::now_ms().wrapping_mul(1_000_003) ^ t0.elapsed().as_nanos() as u64, self.seq.fetch_add(1, Relaxed)));
        let mut prm = self.prm.clone();
        let cap = (budget_ms * 7 / 10).max(1);
        prm.max_millis = Some(prm.max_millis.map_or(cap, |m| m.min(cap)));
        prm.seed = mix(seed, 7);
        let policy = SearchPolicy::new("serve", self.blueprint.clone(), prm);

        let mut hand = Hand::replay(req, Some(&policy), mix(seed, 1))?;
        let street = hand.real.street();

        let mut source = "blueprint";
        let mut reason: Option<String> = None;
        let mut search_ms: Option<f64> = None;
        let mut dist: Option<Dist> = None;
        if let Some(why) = hand.desync.clone() {
            source = "fallback";
            reason = Some(format!("desync: {why}"));
        } else if let Some(mut player) = hand.player.take() {
            let hole = hand.hole;
            let view = hand.view();
            let want_search = opts.search && self.cfg.search;
            if !want_search {
                dist = Some(player.blueprint_dist(&view, hole));
            } else if !player.needs_search(&view) {
                dist = Some(Player::dist(&mut player, &view, hole));
            } else {
                let (d, src, why, ms) = self.guarded_search(&mut player, &view, hole, Duration::from_millis(budget_ms));
                dist = Some(d);
                source = src;
                reason = why;
                search_ms = Some(ms);
            }
        }
        if dist.as_ref().is_some_and(|d| d.is_empty()) {
            dist = None;
            source = "fallback";
            reason = Some("policy returned an empty distribution".into());
        }

        // ---- choose, map to real chips, guard
        let mut notes = std::mem::take(&mut hand.notes);
        let mut ra = match &dist {
            Some(d) => {
                let i = if opts.sample {
                    d.sample(cards::Rng::new(mix(seed, 3)).f64())
                } else {
                    (0..d.len()).max_by(|&a, &b| d.prob(a).total_cmp(&d.prob(b)).then(b.cmp(&a))).unwrap_or(0)
                };
                hand.to_real(d.action(i))
            }
            None => hand.passive(),
        };
        if self.cfg.max_stack_bb > 0.0 {
            let capc = (self.cfg.max_stack_bb * hand.big_blind as f64).round() as i64;
            if let Some((na, note)) = hand.cap_guard(ra, capc) {
                notes.push(note);
                ra = na;
            }
        }
        if !hand.is_legal(ra) {
            notes.push(format!("internal: {ra:?} was not legal; replaced by the passive action"));
            ra = hand.passive();
            source = "fallback";
            reason.get_or_insert_with(|| "illegal mapped action".into());
        }
        if source == "fallback" {
            self.c.fallbacks.fetch_add(1, Relaxed);
        }

        let probs = opts.include_probs.then(|| {
            let mut out: Vec<ProbOut> = vec![];
            if let Some(d) = &dist {
                for i in 0..d.len() {
                    let a = hand.describe(hand.to_real(d.action(i)));
                    let p = d.prob(i);
                    match out.iter_mut().find(|o| o.kind == a.kind && o.to == a.to) {
                        Some(o) => o.p += p,
                        None => out.push(ProbOut { kind: a.kind, to: a.to, p }),
                    }
                }
            }
            for o in out.iter_mut() {
                o.p = (o.p * 1e6).round() / 1e6;
            }
            out.sort_by(|a, b| b.p.total_cmp(&a.p));
            out
        });

        Ok(ActResponse {
            request_id,
            hand_id: req.hand_id.clone(),
            model_id: self.cfg.model_id.clone(),
            street: street_name(street),
            action: hand.describe(ra),
            source,
            fallback_reason: reason,
            probs,
            legal: hand.legal_out(),
            timing: Timing { total_ms: t0.elapsed().as_secs_f64() * 1e3, search_ms, budget_ms },
            notes,
        })
    }

    /// Run the search on a worker thread; blueprint on timeout / panic / no free slot.
    fn guarded_search(&self, player: &mut SearchPlayer, view: &View, hole: [Card; 2], budget: Duration) -> (Dist, &'static str, Option<String>, f64) {
        let t0 = Instant::now();
        let fallback = |player: &mut SearchPlayer, why: String| {
            player.detach_search();
            (player.blueprint_dist(view, hole), "fallback", Some(why), t0.elapsed().as_secs_f64() * 1e3)
        };
        let Some(permit) = Gate::acquire(&self.gate, Duration::from_millis(self.cfg.queue_ms)) else {
            self.c.busy.fetch_add(1, Relaxed);
            return fallback(player, format!("search_busy: all {} search slots in use for {}ms; blueprint", self.gate.capacity(), self.cfg.queue_ms));
        };
        let t_search = Instant::now();
        let mut worker = player.clone();
        let (cfg, state, seat) = (*view.cfg, *view.state, view.seat);
        let (board, history) = (view.board.to_vec(), view.history.to_vec());
        let delay = self.cfg.debug_search_delay_ms;
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new().name("search-decision".into()).spawn(move || {
            let _permit = permit; // held until the search really ends, even if abandoned
            if delay > 0 {
                std::thread::sleep(Duration::from_millis(delay));
            }
            let v = View { cfg: &cfg, state: &state, board: &board, history: &history, seat };
            let d = Player::dist(&mut worker, &v, hole);
            let _ = tx.send(d);
        });
        if let Err(e) = spawned {
            return fallback(player, format!("spawn_failed: {e}"));
        }
        let res = rx.recv_timeout(budget);
        let us = t_search.elapsed().as_micros() as u64;
        self.c.search_micros.fetch_add(us, Relaxed);
        self.c.search_max_micros.fetch_max(us, Relaxed);
        let ms = t_search.elapsed().as_secs_f64() * 1e3;
        match res {
            Ok(d) => {
                self.c.searched.fetch_add(1, Relaxed);
                (d, "search", None, ms)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.c.timeouts.fetch_add(1, Relaxed);
                let (d, s, w, _) = fallback(player, format!("search_timeout>{}ms: blueprint", budget.as_millis()));
                (d, s, w, ms)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.c.panics.fetch_add(1, Relaxed);
                let (d, s, w, _) = fallback(player, "search_panic: blueprint".into());
                (d, s, w, ms)
            }
        }
    }
}
