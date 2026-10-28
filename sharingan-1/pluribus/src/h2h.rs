//! Head-to-head evaluation of two blueprints (any [`PolicyFactory`]): duplicate 6-max matches
//! on identical deals, AIVAT-scored (both sides expose exact probabilities).
//!
//! Three formats, all mbb/game from the first-named side's view, positive = it wins:
//! 1. `3A+3B`: three copies of each, seated in blocks (AAABBB, rotated through all 6 seats)
//!    and alternating (ABABAB, 2 rotations) -- A's edge over B in a mixed table;
//! 2. `1A vs 5B`: A rotates through every seat against five copies of B;
//! 3. `1B vs 5A`: the mirror. If A is the stronger bot, (2) is positive and (3) negative; both
//!    positive means each side beats copies of the other (a non-transitive / abstraction
//!    effect), both negative means each side loses to copies of the other.

use bench::matchrun::{run_match, AivatMode, MatchReport, MatchSpec, Seatings};
use bench::policy::PolicyFactory;
use bench::stats::Est;
use nlhe::GameConfig;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct H2hConfig {
    pub players: usize,
    /// Distinct deals per format (each is replayed under every seating).
    pub deals: u64,
    pub seed: u64,
    pub threads: usize,
}

/// Seatings for `n/2` copies of entrant 0 and the rest entrant 1: all block rotations, plus
/// the two alternating patterns when `n` is even.
pub fn mixed_seatings(n: usize) -> Seatings {
    let base: Vec<usize> = (0..n).map(|s| usize::from(s >= n / 2)).collect();
    let mut all = Seatings::rotations(&base).0;
    if n % 2 == 0 {
        let alt: Vec<usize> = (0..n).map(|s| s % 2).collect();
        all.push(alt.clone());
        all.push(alt.iter().map(|x| 1 - x).collect());
    }
    Seatings(all)
}

pub fn est_json(e: &Est) -> Value {
    json!({"mbb": e.mbb, "se": e.se, "ci95": 1.96 * e.se, "n": e.n})
}

fn part(label: &str, who: &str, r: &MatchReport, secs: f64) -> Value {
    let e = &r.entrants[0];
    let main = e.aivat.unwrap_or(e.raw);
    json!({
        "format": label,
        "first_side": who,
        "mbb": est_json(&main),
        "raw_mbb": est_json(&e.raw),
        "var_ratio_deal": e.var_ratio_deal,
        "by_seat": e.by_seat.iter().enumerate().map(|(s, x)| x.as_ref().map(|x| {
            let est = x.aivat.unwrap_or(x.raw);
            json!({"seat": s, "mbb": est.mbb, "ci95": 1.96 * est.se})
        })).collect::<Vec<_>>(),
        "deals": r.deals, "hands": r.hands, "secs": secs,
    })
}

fn run_one(cfg: GameConfig, entrants: Vec<Arc<dyn PolicyFactory>>, seatings: Seatings, c: &H2hConfig, seed: u64, pool: &rayon::ThreadPool) -> (MatchReport, f64) {
    let mut spec = MatchSpec::new(cfg, entrants, seatings, c.deals, seed);
    spec.aivat = AivatMode::Full;
    let t = Instant::now();
    let r = pool.install(|| run_match(&spec));
    (r, t.elapsed().as_secs_f64())
}

/// Run all three formats; returns the JSON report (see [`print_summary`]).
pub fn h2h(a: Arc<dyn PolicyFactory>, b: Arc<dyn PolicyFactory>, c: &H2hConfig) -> Value {
    let cfg = GameConfig::new(c.players);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(c.threads.max(1)).build().expect("thread pool");
    let (an, bn) = (a.name(), b.name());

    let (r1, t1) = run_one(cfg, vec![a.clone(), b.clone()], mixed_seatings(c.players), c, c.seed, &pool);
    let p1 = part(&format!("{0}A+{0}B", c.players / 2), "A", &r1, t1);
    let (r2, t2) = run_one(cfg, vec![a.clone(), b.clone()], Seatings::hero_vs_field(c.players), c, c.seed ^ 0x1111, &pool);
    let p2 = part(&format!("1A vs {}B", c.players - 1), "A", &r2, t2);
    let (r3, t3) = run_one(cfg, vec![b.clone(), a.clone()], Seatings::hero_vs_field(c.players), c, c.seed ^ 0x2222, &pool);
    let p3 = part(&format!("1B vs {}A", c.players - 1), "B", &r3, t3);
    json!({"a": an, "b": bn, "players": c.players, "deals": c.deals, "seed": c.seed, "threads": c.threads, "results": [p1, p2, p3]})
}

/// Human-readable table of an [`h2h`] report.
pub fn print_summary(rep: &Value) {
    println!("h2h  A = {}\n     B = {}", rep["a"].as_str().unwrap_or("?"), rep["b"].as_str().unwrap_or("?"));
    println!("{} players, {} deals per format, AIVAT mbb/game (95% CI), positive = first side wins\n", rep["players"], rep["deals"]);
    println!("{:<12} {:>5} {:>22} {:>22} {:>8} {:>8}", "format", "side", "AIVAT mbb", "raw mbb", "var-red", "secs");
    for r in rep["results"].as_array().into_iter().flatten() {
        let ci = |v: &Value| format!("{:+8.1} ± {:6.1}", v["mbb"].as_f64().unwrap_or(0.0), v["ci95"].as_f64().unwrap_or(0.0));
        println!(
            "{:<12} {:>5} {:>22} {:>22} {:>7.1}x {:>8.0}",
            r["format"].as_str().unwrap_or("?"),
            r["first_side"].as_str().unwrap_or("?"),
            ci(&r["mbb"]),
            ci(&r["raw_mbb"]),
            r["var_ratio_deal"].as_f64().unwrap_or(0.0),
            r["secs"].as_f64().unwrap_or(0.0)
        );
    }
}
