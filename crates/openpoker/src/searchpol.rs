//! `--policy search:PATH`: a [`SearchPolicy`] over a compact blueprint, wrapped in the
//! time-boxed [`GuardedSearch`]. Mirrors `search/examples/vsbp.rs` (default `SearchParams`,
//! blueprint beliefs, dedicated rayon pool).

use crate::guard::GuardedSearch;
use bench::bp::sizing_by_name;
use blueprint::compact::Compact;
use blueprint::trainer::Bucketer;
use nlhe::abstraction::{BetSizing, TrackOpts};
use nlhe::Street;
use search::bp::Blueprint;
use search::policy::SearchPolicy;
use search::solver::SearchParams;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

pub struct SearchOpts {
    /// Threads of the dedicated search pool.
    pub threads: usize,
    /// Hard per-decision budget; slower searches fall back to the blueprint action.
    pub budget_ms: u64,
    /// `k=v,...` overrides of `SearchParams` (same keys as `vsbp --set`).
    pub sets: String,
    /// Directory of per-flop search buckets (`SearchParams::search_buckets`); unset = off (default config).
    pub search_buckets: Option<String>,
}

impl Default for SearchOpts {
    fn default() -> Self {
        SearchOpts { threads: 16, budget_ms: 5000, sets: String::new(), search_buckets: None }
    }
}

/// Parse `--set` overrides. Errors instead of panicking.
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

/// Build the guarded search factory over the compact blueprint at `path`.
pub fn build(
    path: &Path,
    bucketer: Arc<dyn Bucketer + Send>,
    sizing: BetSizing,
    o: &SearchOpts,
) -> Result<GuardedSearch, String> {
    let compact = Arc::new(Compact::open(path).map_err(|e| format!("compact {}: {e}", path.display()))?);
    let bp = Blueprint::from_source(compact, bucketer, sizing).with_opts(TrackOpts::default());
    finish(bp, o)
}

/// Same, over an already-built blueprint (tests).
pub fn finish(bp: Blueprint, o: &SearchOpts) -> Result<GuardedSearch, String> {
    let mut prm = SearchParams::default();
    apply_sets(&mut prm, &o.sets)?;
    if let Some(dir) = &o.search_buckets {
        let sb = abstraction::search_buckets::SearchBuckets::new(
            Some(dir.into()),
            abstraction::search_buckets::BuildParams::default(),
            64,
        )
        .map_err(|e| format!("search buckets {dir}: {e}"))?;
        prm.search_buckets = Some(Arc::new(sb));
    }
    prm.parallel = true;
    prm.pool = Some(Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(o.threads.max(1))
            .thread_name(|i| format!("search-pool-{i}"))
            .build()
            .map_err(|e| format!("thread pool: {e}"))?,
    ));
    // Cooperative cap well under the hard budget: a slow search returns a truncated (still
    // valid) solution instead of tripping the hard timeout.
    if prm.max_millis.is_none() {
        prm.max_millis = Some((o.budget_ms * 7 / 10).max(1));
    }
    let policy = Arc::new(SearchPolicy::new("search", bp, prm));
    Ok(GuardedSearch::new("search", policy, Duration::from_millis(o.budget_ms)))
}
