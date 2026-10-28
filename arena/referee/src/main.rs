//! `referee` CLI.
//!
//! ```text
//! referee pokerbench  --policy builtin:tag|blueprint:<path> [--sizing local] [--buckets DIR]
//!                     [--split test|train] [--which all|preflop|postflop] [--limit N] [--seed S]
//! referee river-spots --policy ... [--n 50] [--iters 400] [--ranges policy|uniform]
//!                     [--split test|train] [--seed S]
//! ```
//! Both print a JSON summary on stdout.

use rayon::prelude::*;
use referee::pokerbench::{self, Which};
use referee::policies;
use referee::river::{self, RangeMode, RiverSpot};
use serde_json::json;
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

struct Args(BTreeMap<String, String>);
impl Args {
    fn get(&self, k: &str, d: &str) -> String {
        self.0.get(k).cloned().unwrap_or(d.to_string())
    }
    fn num<T: std::str::FromStr>(&self, k: &str, d: T) -> T {
        self.0.get(k).and_then(|v| v.parse().ok()).unwrap_or(d)
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let cmd = argv.get(1).cloned().unwrap_or_default();
    let mut m = BTreeMap::new();
    let mut i = 2;
    while i < argv.len() {
        let k = argv[i].trim_start_matches("--").to_string();
        if i + 1 < argv.len() && !argv[i + 1].starts_with("--") {
            m.insert(k, argv[i + 1].clone());
            i += 2;
        } else {
            m.insert(k, String::new());
            i += 1;
        }
    }
    if let Err(e) = run(&cmd, &Args(m)) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len().max(1) as f64
}

fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    s.get(s.len() / 2).copied().unwrap_or(0.0)
}

fn run(cmd: &str, a: &Args) -> Result<(), String> {
    let data =
        a.0.get("data")
            .map(PathBuf::from)
            .unwrap_or_else(pokerbench::default_dir);
    let test = match a.get("split", "test").as_str() {
        "test" => true,
        "train" => false,
        s => return Err(format!("bad --split {s}")),
    };
    let buckets = a.0.get("buckets").cloned();
    let policy = || {
        policies::parse(
            &a.get("policy", "builtin:tag"),
            &a.get("sizing", "tiny"),
            buckets.as_deref(),
            a.num("capacity", 1usize << 26),
        )
    };
    let seed: u64 = a.num("seed", 1);
    match cmd {
        "pokerbench" => {
            let which = match a.get("which", "all").as_str() {
                "preflop" => Which::Preflop,
                "postflop" => Which::Postflop,
                _ => Which::All,
            };
            let f = policy()?;
            let limit: usize = a.num("limit", 0);
            let (mut res, mut bad_total) = (Vec::new(), 0);
            pokerbench::for_each_chunk(&data, which, test, limit, 20_000, |spots, bad| {
                bad_total += bad;
                res.extend(pokerbench::score_chunk(&spots, &*f, seed));
            })?;
            let rep = pokerbench::report(&f.name(), bad_total, res);
            println!("{}", serde_json::to_string_pretty(&rep).unwrap());
        }
        "river-spots" => {
            let f = policy()?;
            let n: usize = a.num("n", 50);
            let iters: usize = a.num("iters", 400);
            let mode = match a.get("ranges", "policy").as_str() {
                "uniform" => RangeMode::Uniform,
                _ => RangeMode::Policy,
            };
            let (spots, _) = pokerbench::load(&data, Which::Postflop, test)?;
            let mut seen = HashSet::new();
            let mut valid: Vec<RiverSpot> = Vec::new();
            let (mut candidates, mut unusable) = (0, 0);
            for s in spots.iter().filter(|s| s.street == nlhe::Street::River) {
                if !seen.insert(RiverSpot::key(s)) {
                    continue;
                }
                candidates += 1;
                match RiverSpot::from_spot(s) {
                    Ok(r) => valid.push(r),
                    Err(_) => unusable += 1,
                }
            }
            let total = valid.len();
            let pick: Vec<usize> = if n == 0 || n >= total {
                (0..total).collect()
            } else {
                (0..n).map(|i| i * total / n).collect()
            };
            let results: Vec<_> = pick
                .par_iter()
                .map(|&i| river::evaluate(&valid[i], &*f, mode, iters, bench::mix(seed, i as u64)))
                .collect();
            let ok: Vec<_> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
            let col = |g: &dyn Fn(&river::RiverResult) -> f64| -> Vec<f64> {
                ok.iter().map(|r| g(r)).collect()
            };
            let oop = col(&|r| r.ev_loss_mbb[0]);
            let ip = col(&|r| r.ev_loss_mbb[1]);
            let st_oop = col(&|r| r.station_loss_mbb[0]);
            let st_ip = col(&|r| r.station_loss_mbb[1]);
            let both = col(&|r| (r.ev_loss_mbb[0] + r.ev_loss_mbb[1]) / 2.0);
            let st_both = col(&|r| (r.station_loss_mbb[0] + r.station_loss_mbb[1]) / 2.0);
            let pot = col(&|r| r.pot_bb);
            let out = json!({
                "policy": f.name(),
                "ranges": format!("{mode:?}"),
                "iters": iters,
                "river_nodes_in_split": candidates,
                "unusable_nodes": unusable,
                "evaluated": ok.len(),
                "failed": results.len() - ok.len(),
                "ev_loss_mbb": {
                    "mean_both_seats": mean(&both),
                    "median_both_seats": median(&both),
                    "mean_oop": mean(&oop),
                    "mean_ip": mean(&ip),
                    "mean_pct_of_pot": mean(&ok.iter().map(|r| (r.ev_loss_mbb[0] + r.ev_loss_mbb[1]) / 2.0 / 1000.0 / r.pot_bb * 100.0).collect::<Vec<_>>()),
                },
                "control_calling_station_loss_mbb": {
                    "mean_both_seats": mean(&st_both), "mean_oop": mean(&st_oop), "mean_ip": mean(&st_ip),
                },
                "solver_ref_exploitability_mbb_mean": mean(&col(&|r| r.ref_expl_mbb)),
                "mean_pot_bb": mean(&pot),
                "mean_tree_nodes": mean(&col(&|r| r.nodes as f64)),
                "first_spots": ok.iter().take(10).collect::<Vec<_>>(),
            });
            println!("{}", serde_json::to_string_pretty(&out).unwrap());
        }
        _ => {
            println!("usage: referee <pokerbench|river-spots> --policy builtin:tag|blueprint:<path> [--sizing S] [--buckets DIR] [--split test|train] [--limit N | --n N]");
        }
    }
    Ok(())
}
