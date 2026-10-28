//! Quality of the per-flop search buckets vs the blueprint's global 200-bucket tables, on the
//! same flops and exactly the same (hole, board) situations (every situation of the flop, exact
//! equities -- no sampling noise).
//!   cargo run --release -p abstraction --example search_bucket_quality -- \
//!       [--dir data/search-buckets-500] [--bp data/buckets-200] [--flops 16] [--threads 24]
//! Metrics (higher R^2 is better):
//!  * river: R^2 of exact river equity vs a random hand explained by the bucket;
//!  * turn: R^2 of exact turn equity (mean over the 46 rivers) and of its "potential" (std of
//!    the river equity over the 46 rivers: draws are high, made hands low);
//!  * potential-awareness: among random turn-situation pairs with equal equity (|diff| < 0.01) but
//!    clearly different potential (std differs by > 0.06), the fraction put in the SAME bucket (lower = better), and
//!    as a control the same fraction for pairs with equal equity and equal potential (higher = better).
use abstraction::equity::{hole_mask, holes, river_equities, NUM_HOLES};
use abstraction::search_buckets::{BuildParams, SearchBuckets, NUM_FLOPS};
use abstraction::tables::BucketTables;
use cards::{bit, hole_index, mask_of, Card, Rng};
use rayon::prelude::*;
use std::path::Path;
use std::time::Instant;

struct Acc {
    n: Vec<f64>,
    s: Vec<f64>,
    s2: Vec<f64>,
}
impl Acc {
    fn new(k: usize) -> Self {
        Acc { n: vec![0.0; k], s: vec![0.0; k], s2: vec![0.0; k] }
    }
    fn add(&mut self, b: usize, x: f64) {
        self.n[b] += 1.0;
        self.s[b] += x;
        self.s2[b] += x * x;
    }
    fn r2(&self) -> f64 {
        let n: f64 = self.n.iter().sum();
        let m = self.s.iter().sum::<f64>() / n;
        let total = self.s2.iter().sum::<f64>() / n - m * m;
        let within: f64 = (0..self.n.len()).filter(|&b| self.n[b] > 0.0).map(|b| self.s2[b] - self.s[b] * self.s[b] / self.n[b]).sum::<f64>() / n;
        1.0 - within / total
    }
    fn used(&self) -> usize {
        self.n.iter().filter(|&&x| x > 0.0).count()
    }
}

#[derive(Default, Clone)]
struct Tot {
    // river r2, turn eq r2, turn potential r2, buckets used (river, turn) for [search, bp]
    r_r2: [f64; 2],
    t_r2: [f64; 2],
    p_r2: [f64; 2],
    r_used: [f64; 2],
    t_used: [f64; 2],
    diff_pairs: f64,
    diff_same: [f64; 2],
    same_pairs: f64,
    same_same: [f64; 2],
    flops: f64,
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let get = |k: &str, d: &str| a.iter().position(|x| x == &format!("--{k}")).map_or(d.to_string(), |i| a[i + 1].clone());
    let dir = get("dir", "data/search-buckets-500");
    let bpdir = get("bp", "data/buckets-200");
    let nflops: usize = get("flops", "16").parse().unwrap();
    let threads: usize = get("threads", "24").parse().unwrap();
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
    pool.install(|| {
        let mut params = BuildParams::default();
        params.trees = get("trees", &params.trees.to_string()).parse().unwrap();
        params.turn_fit_sample = get("turn-fit", &params.turn_fit_sample.to_string()).parse().unwrap();
        params.turn_iters = get("turn-iters", &params.turn_iters.to_string()).parse().unwrap();
        params.river_fit_sample = get("river-fit", &params.river_fit_sample.to_string()).parse().unwrap();
        params.river_iters = get("river-iters", &params.river_iters.to_string()).parse().unwrap();
        let sb = SearchBuckets::new(Some(dir.into()), params, 4).unwrap();
        let bp = BucketTables::load(Path::new(&bpdir)).unwrap();
        let hs = holes();
        let mut tot = Tot::default();
        for fi in 0..nflops {
            let id = (fi * NUM_FLOPS / nflops) as u16;
            let t0 = Instant::now();
            let fb = sb.flop_buckets(id);
            let load_s = t0.elapsed().as_secs_f64();
            let flop = fb.flop;
            let fm = mask_of(&flop);
            let live: Vec<Card> = (0..52u8).filter(|&c| bit(c) & fm == 0).collect();
            let mut pairs = vec![];
            for (i, &t) in live.iter().enumerate() {
                for &r in &live[i + 1..] {
                    pairs.push((t, r));
                }
            }
            // (hole, eq, search river bucket, blueprint river bucket) for every situation
            let per: Vec<Vec<(u16, f32, u16, u16)>> = pairs
                .par_iter()
                .map(|&(t, r)| {
                    let board = fm | bit(t) | bit(r);
                    let mut eq = [0f32; NUM_HOLES];
                    river_equities(board, &mut eq);
                    let bc = [flop[0], flop[1], flop[2], t, r];
                    hs.iter()
                        .enumerate()
                        .filter(|(_, h)| hole_mask(**h) & board == 0)
                        .map(|(i, &h)| (i as u16, eq[i], fb.river(h, t, r), bp.bucket(h, &bc) as u16))
                        .collect()
                })
                .collect();
            let mut rs = Acc::new(fb.k_river as usize);
            let mut rb = Acc::new(bp.buckets[2] as usize);
            let mut tsum = vec![0f64; NUM_HOLES * 52];
            let mut tsq = vec![0f64; NUM_HOLES * 52];
            for (&(t, r), v) in pairs.iter().zip(&per) {
                for &(h, e, sbk, bbk) in v {
                    let e = e as f64;
                    rs.add(sbk as usize, e);
                    rb.add(bbk as usize, e);
                    for tc in [t, r] {
                        tsum[h as usize * 52 + tc as usize] += e;
                        tsq[h as usize * 52 + tc as usize] += e * e;
                    }
                }
            }
            drop(per);
            let mut ts = [Acc::new(fb.k_turn as usize), Acc::new(fb.k_turn as usize)];
            let mut tb = [Acc::new(bp.buckets[1] as usize), Acc::new(bp.buckets[1] as usize)];
            let mut sit: Vec<(f32, f32, u16, u16)> = vec![]; // eq, potential, search bucket, bp bucket
            for &t in &live {
                for (i, &h) in hs.iter().enumerate() {
                    if hole_mask(h) & (fm | bit(t)) != 0 {
                        continue;
                    }
                    let e = tsum[i * 52 + t as usize] / 46.0;
                    let sd = (tsq[i * 52 + t as usize] / 46.0 - e * e).max(0.0).sqrt();
                    let sbk = fb.turn(h, t);
                    let bbk = bp.bucket(h, &[flop[0], flop[1], flop[2], t]) as u16;
                    ts[0].add(sbk as usize, e);
                    ts[1].add(sbk as usize, sd);
                    tb[0].add(bbk as usize, e);
                    tb[1].add(bbk as usize, sd);
                    sit.push((e as f32, sd as f32, sbk, bbk));
                }
            }
            let mut rng = Rng::new(id as u64 + 1);
            let (mut dp, mut ds, mut db, mut sp, mut ss, mut sbb) = (0f64, 0f64, 0f64, 0f64, 0f64, 0f64);
            for _ in 0..2_000_000 {
                let (x, y) = (sit[rng.below(sit.len() as u32) as usize], sit[rng.below(sit.len() as u32) as usize]);
                if (x.0 - y.0).abs() < 0.01 {
                    let dpot = (x.1 - y.1).abs();
                    if dpot > 0.06 {
                        dp += 1.0;
                        ds += (x.2 == y.2) as u32 as f64;
                        db += (x.3 == y.3) as u32 as f64;
                    } else if dpot < 0.01 {
                        sp += 1.0;
                        ss += (x.2 == y.2) as u32 as f64;
                        sbb += (x.3 == y.3) as u32 as f64;
                    }
                }
            }
            let _ = hole_index(0, 1);
            println!(
                "flop {id:4}: river R2 search {:.4} bp {:.4} | turn eq R2 search {:.4} bp {:.4} | turn potential R2 search {:.4} bp {:.4} | equal-eq/diff-draw same-bucket search {:.3} bp {:.3} ({} pairs) | control same-bucket search {:.3} bp {:.3} | used buckets river search {} bp {} | turn search {}/{} bp {}/{} | load {:.2}s",
                rs.r2(), rb.r2(), ts[0].r2(), tb[0].r2(), ts[1].r2(), tb[1].r2(), ds / dp.max(1.0), db / dp.max(1.0), dp, ss / sp.max(1.0), sbb / sp.max(1.0),
                rs.used(), rb.used(), ts[0].used(), ts[0].n.len(), tb[0].used(), tb[0].n.len(), load_s
            );
            tot.r_r2[0] += rs.r2();
            tot.r_r2[1] += rb.r2();
            tot.t_r2[0] += ts[0].r2();
            tot.t_r2[1] += tb[0].r2();
            tot.p_r2[0] += ts[1].r2();
            tot.p_r2[1] += tb[1].r2();
            tot.r_used[0] += rs.used() as f64;
            tot.r_used[1] += rb.used() as f64;
            tot.t_used[0] += ts[0].used() as f64;
            tot.t_used[1] += tb[0].used() as f64;
            tot.diff_pairs += dp;
            tot.diff_same[0] += ds;
            tot.diff_same[1] += db;
            tot.same_pairs += sp;
            tot.same_same[0] += ss;
            tot.same_same[1] += sbb;
            tot.flops += 1.0;
        }
        let n = tot.flops;
        println!("MEAN over {n} flops:");
        println!("  river equity R2        search-500 {:.4}  blueprint-200 {:.4}", tot.r_r2[0] / n, tot.r_r2[1] / n);
        println!("  turn equity R2         search-500 {:.4}  blueprint-200 {:.4}", tot.t_r2[0] / n, tot.t_r2[1] / n);
        println!("  turn potential R2      search-500 {:.4}  blueprint-200 {:.4}", tot.p_r2[0] / n, tot.p_r2[1] / n);
        println!("  equal-equity/different-draw pairs in same bucket (lower better; {} pairs): search-500 {:.4}  blueprint-200 {:.4}", tot.diff_pairs, tot.diff_same[0] / tot.diff_pairs.max(1.0), tot.diff_same[1] / tot.diff_pairs.max(1.0));
        println!("  control equal-equity/equal-draw pairs in same bucket ({} pairs): search-500 {:.4}  blueprint-200 {:.4}", tot.same_pairs, tot.same_same[0] / tot.same_pairs.max(1.0), tot.same_same[1] / tot.same_pairs.max(1.0));
        println!("  buckets used per flop: river search {:.0} bp {:.0}; turn search {:.0} bp {:.0}", tot.r_used[0] / n, tot.r_used[1] / n, tot.t_used[0] / n, tot.t_used[1] / n);
    });
}
