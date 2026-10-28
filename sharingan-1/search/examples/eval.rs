//! Search vs blueprint evaluation: duplicate matches with AIVAT, LBR, per-decision timing.
//!
//! cargo run --release -p search --example eval -- --players 2 --bp-iters 3000000 \
//!     --deals 2000 --search-iters 20000 --lbr-deals 300
//!
//! Flags: --players N, --bp-iters N, --sizing tiny|small, --bucketer tables|coarse,
//! --deals N, --search-iters N, --seed S, --lbr-deals N, --lbr-samples N, --final|--average,
//! --leaf cont4|blueprint, --no-lbr, --bp-cache PATH (save/load the trained blueprint),
//! --focus F, --par-search (parallel solver inside matches; timing probes only), --hero-only-seat0 (6-max: one rotation instead of six).
use bench::bp::BlueprintPolicy;
use bench::lbr::{run_lbr, LbrConfig};
use bench::matchrun::{run_match, AivatMode, MatchSpec, Seatings};
use bench::policy::PolicyFactory;
use blueprint::store::Store;
use nlhe::abstraction::{BetSizing, Menu};
use nlhe::GameConfig;
use search::bp::Blueprint;
use search::policy::SearchPolicy;
use search::solver::{LeafMode, SearchParams};
use search::testkit::{best_bucketer, toy_schedule};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

fn sizing(name: &str) -> BetSizing {
    let m = Menu::new;
    match name {
        "tiny" => BetSizing::tiny(),
        "local" => pluribus::sizing("local"),
        "small" => {
            let pre = [m(&[0.5, 1.0, 2.0], true), m(&[1.0], true), m(&[], true)];
            let post = [m(&[0.5, 1.0], true), m(&[1.0], true)];
            BetSizing::new([&pre, &post, &post, &post], [3, 3, 3, 3])
        }
        o => panic!("sizing {o}"),
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
    let players: usize = get("players", "2").parse().unwrap();
    let bp_iters: u64 = get("bp-iters", "3000000").parse().unwrap();
    let deals: u64 = get("deals", "2000").parse().unwrap();
    let seed: u64 = get("seed", "1").parse().unwrap();
    let siters: u64 = get("search-iters", "20000").parse().unwrap();
    let sizing_name = get("sizing", "tiny");
    let cfg = GameConfig::new(players);

    let (bucketer, bname) = if get("bucketer", "tables") == "coarse" {
        (Arc::new(bench::bp::CoarseBucketer) as Arc<dyn blueprint::trainer::Bucketer + Send>, "coarse")
    } else {
        best_bucketer()
    };
    let bsz = sizing(&sizing_name);
    let cache = kv.get("bp-cache").map(PathBuf::from);
    let t0 = Instant::now();
    let store = if let Some(p) = cache.as_ref().filter(|p| p.exists()) {
        eprintln!("loading blueprint {}", p.display());
        Arc::new(blueprint::io::load(p, 1 << 27, 1 << 24).expect("load"))
    } else {
        eprintln!("training blueprint: {players}p {sizing_name} {bname} {bp_iters} iters");
        macro_rules! train {
            ($b:expr) => {{
                let mut tr = blueprint::trainer::Trainer::new(cfg, bsz, toy_schedule(bp_iters), $b, 0, seed);
                tr.store = Arc::new(Store::with_index(1 << 27, 1 << 24));
                tr.run(0, bp_iters, |t, i, s| {
                    if i % 1_000_000 < 4096 {
                        eprintln!("  it {i} {:.0}s infosets {}", s, t.store.infosets());
                    }
                });
                std::mem::replace(&mut tr.store, Arc::new(Store::new(16)))
            }};
        }
        // Concrete type needed by Trainer: wrap the dyn bucketer.
        struct Dyn(Arc<dyn blueprint::trainer::Bucketer + Send>);
        impl blueprint::trainer::Bucketer for Dyn {
            fn bucket(&self, s: nlhe::Street, h: [cards::Card; 2], b: &[cards::Card]) -> u32 {
                self.0.bucket(s, h, b)
            }
            fn n_buckets(&self, s: nlhe::Street) -> u32 {
                self.0.n_buckets(s)
            }
        }
        let d = Dyn(bucketer.clone());
        let st = train!(&d);
        if let Some(p) = &cache {
            blueprint::io::save(&st, p).expect("save");
        }
        st
    };
    eprintln!("blueprint ready in {:.0}s: {} infosets", t0.elapsed().as_secs_f64(), store.infosets());
    let bp = Blueprint::new(store.clone(), bucketer.clone(), bsz);

    let mut prm = SearchParams { iters: siters, seed, ..Default::default() };
    prm.play_final = kv.contains_key("final");
    // Matches already run one deal per core; nested rayon only inflates per-search latency.
    prm.parallel = kv.contains_key("par-search");
    prm.postflop_search = !kv.contains_key("no-search");
    prm.vector = get("vector", "0") != "0";
    prm.vec_plus = !kv.contains_key("no-plus");
    prm.vec_pooled = kv.contains_key("vec-pooled");
    if let Some(d) = kv.get("search-buckets") {
        let sb = abstraction::search_buckets::SearchBuckets::new(Some(PathBuf::from(d)), abstraction::search_buckets::BuildParams::default(), 64).expect("search buckets");
        prm.search_buckets = Some(Arc::new(sb));
    }
    prm.search_flop = !kv.contains_key("no-flop");
    prm.search_turn = !kv.contains_key("no-turn");
    prm.search_river = !kv.contains_key("no-river");
    prm.blueprint_beliefs = kv.contains_key("bp-beliefs");
    if let Some(v) = kv.get("vec-iters") {
        let x: Vec<u64> = v.split(',').map(|y| y.parse().unwrap()).collect();
        prm.vec_iters = [x[0], x[1], x[2]];
    }
    if let Some(v) = kv.get("vec-min") {
        prm.vec_min_street = match v.as_str() {
            "flop" => nlhe::Street::Flop,
            "turn" => nlhe::Street::Turn,
            _ => nlhe::Street::River,
        };
    }
    if let Some(v) = kv.get("warm") {
        prm.warm_start = v.parse().unwrap();
    }
    if let Some(v) = kv.get("prune") {
        prm.prune_prob = v.parse().unwrap();
    }
    if get("leaf", "cont4") == "blueprint" {
        prm.leaf = LeafMode::Blueprint;
    }
    if let Some(f) = kv.get("focus") {
        prm.focus_own = f.parse().unwrap();
    }
    if let Some(ms) = kv.get("max-ms") {
        prm.max_millis = Some(ms.parse().unwrap());
    }
    let search = Arc::new(SearchPolicy::new(
        &format!("search{}{}", siters, if prm.play_final { "F" } else { "A" }),
        bp.clone(),
        prm,
    ));
    let bpp: Arc<dyn PolicyFactory> = Arc::new(BlueprintPolicy::new("blueprint", store.clone(), bucketer.clone(), bsz));
    let sp: Arc<dyn PolicyFactory> = search.clone();

    // Duplicate match with AIVAT.
    let seatings = if players == 2 || !kv.contains_key("hero-only-seat0") {
        Seatings::hero_vs_field(players)
    } else {
        Seatings(vec![{
            let mut v = vec![1; players];
            v[0] = 0;
            v
        }])
    };
    let mut spec = MatchSpec::new(cfg, vec![sp.clone(), bpp.clone()], seatings, deals, seed + 100);
    spec.aivat = AivatMode::Full;
    let t = Instant::now();
    let rep = run_match(&spec);
    let el = t.elapsed().as_secs_f64();
    println!("== {players}-player duplicate match: {} vs {} blueprint(s), {} deals, {el:.0}s ==", sp.name(), players - 1, deals);
    rep.print();
    let h = &rep.entrants[0];
    if let Some(av) = h.aivat {
        println!("RESULT search-vs-blueprint AIVAT {av} mbb/game (95% CI {:?})", av.ci95());
    }
    let s = search.stats();
    println!(
        "timing: {} searches, mean {:.1} ms, max {:.0} ms, mean iters {:.0}, mean infosets {:.0}, resolves {}, preflop {}, max vector storage {:.0} MB",
        s.searches, s.mean_ms, s.max_ms, s.mean_iters, s.mean_infosets, s.resolves, s.preflop, s.max_vec_mb
    );

    if !kv.contains_key("no-lbr") {
        let ld: u64 = get("lbr-deals", "300").parse().unwrap();
        let lc = LbrConfig { samples: get("lbr-samples", "300").parse().unwrap(), ..Default::default() };
        let t = Instant::now();
        let lb = run_lbr(&cfg, &lc, bpp.clone(), ld, seed + 7);
        println!("{lb}   [{:.0}s]", t.elapsed().as_secs_f64());
        let t = Instant::now();
        let ls = run_lbr(&cfg, &lc, sp.clone(), ld, seed + 7);
        println!("{ls}   [{:.0}s]", t.elapsed().as_secs_f64());
        let s = search.stats();
        println!("search stats after LBR: {} searches, mean {:.1} ms", s.searches, s.mean_ms);
    }
}
