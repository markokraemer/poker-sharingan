//! Scripted off-menu probes vs the blueprint (legacy and shadow tracking) and vs the search agent.
//!
//! cargo run --release -p search --example probe -- --players 2 --bp-cache data/search-bp/hu_tiny_3m.bin \
//!     --deals 4000 --search-iters 2000 --probes "probe:bet=0.3|probe:bet=3|probe:pre=3"
//!
//! For every probe spec (see `bench::probe`) prints the probe's AIVAT result (mbb/game, positive =
//! the probe wins) against 1 / players-1 copies of: blueprint with legacy tracking, blueprint
//! with shadow tracking (`TrackOpts::default()`), search.
use bench::bp::BlueprintPolicy;
use bench::matchrun::{run_match, AivatMode, MatchSpec, Seatings};
use bench::policy::PolicyFactory;
use nlhe::abstraction::{BetSizing, TrackOpts};
use nlhe::GameConfig;
use search::bp::Blueprint;
use search::policy::SearchPolicy;
use search::solver::SearchParams;
use search::testkit::best_bucketer;
use std::collections::HashMap;
use std::sync::Arc;

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
    let deals: u64 = get("deals", "4000").parse().unwrap();
    let siters: u64 = get("search-iters", "2000").parse().unwrap();
    let cfg = GameConfig::new(players);
    let (bucketer, _) = best_bucketer();
    let sizing = BetSizing::tiny();
    let store = Arc::new(blueprint::io::load(std::path::Path::new(&get("bp-cache", "data/search-bp/hu_tiny_3m.bin")), 1 << 27, 1 << 24).expect("load"));
    let mk = |o: TrackOpts, n: &str| -> Arc<dyn PolicyFactory> {
        Arc::new(BlueprintPolicy::new(n, store.clone(), bucketer.clone(), sizing).with_opts(o))
    };
    let fixed = TrackOpts::default();
    let search_prm = SearchParams { iters: siters, play_final: kv.contains_key("final"), parallel: false, ..Default::default() };
    let search: Arc<dyn PolicyFactory> = Arc::new(SearchPolicy::new("search", Blueprint::new(store.clone(), bucketer.clone(), sizing), search_prm));
    let entrants: Vec<(&str, Arc<dyn PolicyFactory>)> = vec![
        ("bp-legacy", mk(TrackOpts::LEGACY, "bp-legacy")),
        ("bp-shadow", mk(fixed, "bp-shadow")),
        ("search", search),
    ];
    let only = get("only", "");
    println!("{:<26}{}", "probe (probe's mbb/game)", entrants.iter().map(|e| format!("{:<26}", e.0)).collect::<String>());
    for spec in get("probes", "probe:|probe:bet=0.3|probe:bet=3|probe:pre=3|probe:pre=8").split('|') {
        let probe = bench::probe::parse(spec).expect("probe");
        let mut row = format!("{spec:<26}");
        for (name, e) in &entrants {
            if !only.is_empty() && !only.split(',').any(|o| o == *name) {
                row += &format!("{:<26}", "-");
                continue;
            }
            let mut m = MatchSpec::new(cfg, vec![probe.clone(), e.clone()], Seatings::hero_vs_field(players), deals, 21);
            m.aivat = AivatMode::Full;
            let r = run_match(&m);
            let h = &r.entrants[0];
            let est = h.aivat.unwrap_or(h.raw);
            row += &format!("{:<26}", format!("{:+.0} ± {:.0}", est.mbb, 1.96 * est.se));
        }
        println!("{row}");
    }
}
