//! Search / probe / blueprint matches against a COMPACT blueprint (real final blueprint B).
//!
//! cargo run --release -p search --example vsbp -- --compact data/eval/B-.../compact.plcp \
//!     --buckets data/buckets-200 --sizing local --players 6 --hero 'probe:bet=3' --field search --deals 500
//!
//! `--hero` / `--field`: `search` (SearchPolicy, default config unless --search-iters N etc.), `B`
//! (blueprint with shadow tracking), `B-legacy`, a `probe:` spec, a builtin (`random|station|nit|tag|lag|maniac`), or an external bot (`chart6`, `chart6-loose`, `chart6-tight`, `rlcard:<ckpt>`, `ext:<name>=<json argv>`; --py/--ext-dir/--ext-workers/--ext-timeout-ms). Hero sits in every seat in turn
//! against `players-1` copies of the field; duplicate cards, AIVAT. Positive = hero wins.
//! Extra flags: --seed S, --search-iters N, --no-search (SearchPolicy plays blueprint only), --legacy-search.
use bench::bp::Source;
use bench::compact::CompactPolicy;
use bench::matchrun::{run_match, AivatMode, MatchSpec, Seatings};
use bench::policy::PolicyFactory;
use blueprint::compact::Compact;
use nlhe::abstraction::TrackOpts;
use nlhe::GameConfig;
use search::bp::Blueprint;
use search::policy::SearchPolicy;
use search::solver::SearchParams;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// `--set k=v,k=v`: SearchParams overrides (iters, warm, focus, leaf=cont4|bp, final, vector, vecmin=flop|turn|river,
/// vecit=a/b/c, plus, pooled, flop, turn, river, pre (bools 0/1), bpbeliefs, prune, sizing=small|default, later=small,
/// sbuckets=<dir>, pmax, mvec (multiway vector), mvecit, ...).
fn apply_sets(prm: &mut SearchParams, sets: &str) {
    use nlhe::Street;
    for kv in sets.split(',').filter(|x| !x.is_empty()) {
        let (k, v) = kv.split_once('=').unwrap_or((kv, "1"));
        let b = || v != "0" && v != "false";
        match k {
            "iters" => prm.iters = v.parse().unwrap(),
            "warm" => prm.warm_start = v.parse().unwrap(),
            "focus" => prm.focus_own = v.parse().unwrap(),
            "leaf" => prm.leaf = if v == "bp" { search::solver::LeafMode::Blueprint } else { search::solver::LeafMode::Cont4 },
            "final" => prm.play_final = b(),
            "vector" => prm.vector = b(),
            "vecmin" => prm.vec_min_street = match v { "flop" => Street::Flop, "turn" => Street::Turn, _ => Street::River },
            "vecit" => {
                let x: Vec<u64> = v.split('/').map(|y| y.parse().unwrap()).collect();
                prm.vec_iters = [x[0], x[1], x[2]];
            }
            "plus" => prm.vec_plus = b(),
            "pooled" => prm.vec_pooled = b(),
            "flop" => prm.search_flop = b(),
            "turn" => prm.search_turn = b(),
            "river" => prm.search_river = b(),
            "pre" => prm.preflop_search = b(),
            "post" => prm.postflop_search = b(),
            "bpbeliefs" => prm.blueprint_beliefs = b(),
            "maxp" => {
                let x: Vec<usize> = v.split('/').map(|y| y.parse().unwrap()).collect();
                prm.max_players = [x[0], x[1], x[2]];
            }
            "minflop" => prm.min_flop_players = v.parse().unwrap(),
            "offbel" => prm.offtree_search_beliefs = b(),
            "opppool" => prm.opp_pool = b(),
            "ownbel" => prm.own_search_beliefs = b(),
            "tremble" => prm.belief_tremble = v.parse().unwrap(),
            "prune" => prm.prune_prob = v.parse().unwrap(),
            "ms" => prm.max_millis = Some(v.parse().unwrap()),
            "sizing" if v == "small" => prm.sizing = search::tree::small_sizing(),
            "sizing" if v == "local" => prm.sizing = pluribus::sizing("local"),
            "later" if v == "local" => prm.later_sizing = pluribus::sizing("local"),
            "later" if v == "small" => prm.later_sizing = search::tree::small_sizing(),
            "sbuckets" => prm.search_buckets = Some(Arc::new(abstraction::search_buckets::SearchBuckets::new(Some(std::path::PathBuf::from(v)), abstraction::search_buckets::BuildParams::default(), 64).expect("sbuckets"))),
            _ => panic!("unknown --set key {k}"),
        }
    }
}

fn main() {
    let mut kv: HashMap<String, String> = HashMap::new();
    let a: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < a.len() {
        let k = a[i].trim_start_matches("--").to_string();
        if i + 1 < a.len() && !a[i + 1].starts_with("--") {
            kv.insert(k, a[i + 1].clone());
            i += 2;
        } else {
            kv.insert(k, "true".into());
            i += 1;
        }
    }
    let get = |k: &str, d: &str| kv.get(k).cloned().unwrap_or(d.into());
    let players: usize = get("players", "6").parse().unwrap();
    let deals: u64 = get("deals", "500").parse().unwrap();
    let seed: u64 = get("seed", "11").parse().unwrap();
    let cfg = GameConfig::new(players);
    let sizing = pluribus::sizing(&get("sizing", "local"));
    let bucketer = bench::bp::bucketer_by_name(&format!("tables:{}", get("buckets", "data/buckets-200"))).expect("buckets");
    let path = get("compact", "");
    let compact = Arc::new(Compact::open(std::path::Path::new(&path)).expect("compact"));
    let opts = if kv.contains_key("legacy-search") { TrackOpts::LEGACY } else { TrackOpts::default() };

    // Matches already run one deal per core: no nested parallelism inside a search.
    let mut prm = SearchParams { parallel: false, ..Default::default() };
    if let Some(v) = kv.get("search-iters") {
        prm.iters = v.parse().unwrap();
    }
    if let Some(t) = kv.get("par-threads") {
        // Latency mode: run with RAYON_NUM_THREADS=1 (deals sequential), searches use this pool.
        prm.parallel = true;
        prm.pool = Some(Arc::new(rayon::ThreadPoolBuilder::new().num_threads(t.parse().unwrap()).build().unwrap()));
    }
    if let Some(sets) = kv.get("set") {
        apply_sets(&mut prm, sets);
    }
    if kv.contains_key("no-search") {
        prm.postflop_search = false;
        prm.preflop_search = false;
    }
    let src: Arc<dyn Source> = compact.clone();
    let bp = Blueprint::from_source(src, bucketer.clone(), sizing).with_opts(opts);
    let search = Arc::new(SearchPolicy::new("search", bp, prm));
    let b_fixed = Arc::new(CompactPolicy::new("B", compact.clone(), bucketer.clone(), sizing));
    let b_legacy = Arc::new(CompactPolicy::new("B-legacy", compact.clone(), bucketer.clone(), sizing).with_opts(TrackOpts::LEGACY));

    let mk = |s: &str| -> Arc<dyn PolicyFactory> {
        match s {
            "search" => search.clone(),
            "B" => b_fixed.clone(),
            "B-legacy" => b_legacy.clone(),
            p if p.starts_with("probe:") => bench::probe::parse(p).expect("probe"),
            p if bench::builtin::builtin(p).is_some() => bench::builtin::builtin(p).unwrap(),
            o => {
                // chart6 / chart6-loose / chart6-tight / rlcard:<ckpt> / ext:<name>=<json argv>
                let home = std::env::var("HOME").unwrap_or_default();
                let py = get("py", &format!("{home}/extbots/rlcard-env/venv/bin/python"));
                let dir = get("ext-dir", "scripts/extbots");
                let tmo = std::time::Duration::from_millis(get("ext-timeout-ms", "10000").parse().unwrap());
                match bench::ext::parse_external(o, &py, &dir, get("ext-workers", "16").parse().unwrap(), tmo) {
                    Some(r) => r.expect("external spec"),
                    None => panic!("unknown policy {o}"),
                }
            }
        }
    };
    let (hero, field) = (mk(&get("hero", "search")), mk(&get("field", "B")));
    let mut m = MatchSpec::new(cfg, vec![hero.clone(), field.clone()], Seatings::hero_vs_field(players), deals, seed);
    m.aivat = AivatMode::Full;
    let t = Instant::now();
    let r = run_match(&m);
    let el = t.elapsed().as_secs_f64();
    let h = &r.entrants[0];
    let est = h.aivat.unwrap_or(h.raw);
    println!(
        "RESULT hero={} field={} players={players} deals={deals} hands={} raw {} | aivat {:+.1} +- {:.1} mbb/game  [{el:.0}s]",
        get("hero", "search"),
        get("field", "B"),
        deals * players as u64,
        h.raw,
        est.mbb,
        1.96 * est.se
    );
    let s = search.stats();
    println!(
        "search timing: {} searches, mean {:.1} ms, max {:.0} ms, mean iters {:.0}, resolves {}, preflop {}, infosets mean {:.0} max {}",
        s.searches, s.mean_ms, s.max_ms, s.mean_iters, s.resolves, s.preflop, s.mean_infosets, s.max_infosets
    );
    for p in [&hero, &field] {
        if let Some(rep) = p.report() {
            println!("report {}: {rep}", p.name());
        }
    }
}
