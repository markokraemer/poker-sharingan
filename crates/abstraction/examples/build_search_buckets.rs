//! Offline build of the per-flop search buckets (500 turn + 500 river per flop).
//!   cargo run --release -p abstraction --example build_search_buckets -- \
//!       [--out data/search-buckets-500] [--threads 24] [--limit N] [--stride S] [--tiny]
//! Resumable: flops whose file already exists are skipped. `--stride S` builds every S-th flop
//! (for timing / quality sampling), `--limit N` stops after N flops.
use abstraction::search_buckets::{BuildParams, BuildStats, SearchBuckets, NUM_FLOPS};
use rayon::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::Instant;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let get = |k: &str, d: &str| a.iter().position(|x| x == &format!("--{k}")).map_or(d.to_string(), |i| a[i + 1].clone());
    let out = get("out", "data/search-buckets-500");
    let threads: usize = get("threads", "24").parse().unwrap();
    let limit: usize = get("limit", "100000").parse().unwrap();
    let stride: usize = get("stride", "1").parse().unwrap();
    let mut params = if a.iter().any(|x| x == "--tiny") { BuildParams::tiny() } else { BuildParams::default() };
    params.trees = get("trees", &params.trees.to_string()).parse().unwrap();
    params.turn_fit_sample = get("turn-fit", &params.turn_fit_sample.to_string()).parse().unwrap();
    params.turn_iters = get("turn-iters", &params.turn_iters.to_string()).parse().unwrap();
    params.river_fit_sample = get("river-fit", &params.river_fit_sample.to_string()).parse().unwrap();
    params.river_iters = get("river-iters", &params.river_iters.to_string()).parse().unwrap();
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
    pool.install(|| {
        let t0 = Instant::now();
        let sb = SearchBuckets::new(Some(out.clone().into()), params, 1).unwrap();
        eprintln!("opponent clusters ready {:.1}s; building into {out} with {threads} threads", t0.elapsed().as_secs_f64());
        let ids: Vec<u16> = (0..NUM_FLOPS as u16).step_by(stride).filter(|&i| !sb.flop_path(i).unwrap().exists()).take(limit).collect();
        let done = AtomicUsize::new(0);
        let agg = Mutex::new((BuildStats::default(), 0f64));
        let t1 = Instant::now();
        // Flops are built concurrently (outer parallelism); inner k-means parallelism shares the pool.
        ids.par_iter().for_each(|&id| {
            let t = Instant::now();
            let (fb, st) = sb.build_id(id);
            fb.write(&sb.flop_path(id).unwrap()).unwrap();
            let wall = t.elapsed().as_secs_f64();
            {
                let mut g = agg.lock().unwrap();
                g.0.river_features_s += st.river_features_s;
                g.0.river_cluster_s += st.river_cluster_s;
                g.0.turn_features_s += st.turn_features_s;
                g.0.turn_cluster_s += st.turn_cluster_s;
                g.0.tree_corr += st.tree_corr;
                g.1 += wall;
            }
            let n = done.fetch_add(1, Relaxed) + 1;
            if n % 10 == 0 || n == ids.len() {
                eprintln!("[{n}/{}] flop {id} wall {wall:.1}s (river feat {:.1} clus {:.1}, turn feat {:.1} clus {:.1}, tree corr {:.3}); elapsed {:.0}s", ids.len(), st.river_features_s, st.river_cluster_s, st.turn_features_s, st.turn_cluster_s, st.tree_corr, t1.elapsed().as_secs_f64());
            }
        });
        let (s, wall_sum) = &*agg.lock().unwrap();
        let n = ids.len().max(1) as f64;
        println!(
            "built {} flops in {:.0}s total ({:.2}s/flop amortised on {threads} threads); mean per-flop wall {:.1}s; mean stage s: river feat {:.1} cluster {:.1}, turn feat {:.1} cluster {:.1}; mean tree corr {:.3}",
            ids.len(), t1.elapsed().as_secs_f64(), t1.elapsed().as_secs_f64() / n, wall_sum / n,
            s.river_features_s / n, s.river_cluster_s / n, s.turn_features_s / n, s.turn_cluster_s / n, s.tree_corr as f64 / n
        );
    });
}
