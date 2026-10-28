//! Full-scale check of a compact file against the reference semantics: load the checkpoint
//! into a `Store`, apply `snapshot::export_in_place` (with the same sums file), and compare
//! `Store::strategy` of EVERY infoset with the compact file's distribution.
//!
//! cargo run --release -p blueprint --example compact_verify -- CKPT COMPACT [SUMS] [THREADS]
//! Needs RAM for the whole store (≈ 36 B per infoset): use it on checkpoints that fit.
use blueprint::compact::Compact;
use blueprint::store::Slot;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (ck, cp) = (Path::new(&a[1]), Path::new(&a[2]));
    let sums = a.get(3).filter(|s| !s.is_empty() && s.as_str() != "-").map(|s| Path::new(s.as_str()));
    let threads: usize = a.get(4).map_or(8, |s| s.parse().unwrap());
    rayon::ThreadPoolBuilder::new().num_threads(threads).build_global().unwrap();
    let t = Instant::now();
    let store = blueprint::io::load(ck, 0, 0).unwrap();
    eprintln!("loaded {} infosets in {:.0}s", store.infosets(), t.elapsed().as_secs_f64());
    let t = Instant::now();
    blueprint::snapshot::export_in_place(&store, sums).unwrap();
    eprintln!("export_in_place (sums: {}) in {:.0}s", sums.is_some(), t.elapsed().as_secs_f64());
    let c = Compact::open(cp).unwrap();
    assert_eq!(c.len(), store.infosets(), "infoset count");
    let (bad, missing, n) = (AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0));
    let worst = AtomicU64::new(0); // max error in 1e-6 units
    let t = Instant::now();
    store.par_scan(0, store.table_len(), |key, slot: Slot| {
        n.fetch_add(1, Relaxed);
        let Some(i) = c.find(key) else {
            missing.fetch_add(1, Relaxed);
            return;
        };
        let q = c.entry(i);
        let k = slot.n_actions as usize;
        if q.len() != k || q.iter().map(|&x| x as u32).sum::<u32>() != 255 {
            bad.fetch_add(1, Relaxed);
            return;
        }
        let mut p = [0f32; 64];
        store.strategy(slot, &mut p[..k]);
        let e = (0..k).map(|j| (q[j] as f32 / 255.0 - p[j]).abs()).fold(0f32, f32::max);
        worst.fetch_max((e * 1e6) as u64, Relaxed);
        if e > 1.0 / 255.0 + 1e-4 {
            bad.fetch_add(1, Relaxed);
        }
    });
    eprintln!("compared {} infosets in {:.0}s: {} missing, {} bad, worst error {:.5}", n.load(Relaxed), t.elapsed().as_secs_f64(), missing.load(Relaxed), bad.load(Relaxed), worst.load(Relaxed) as f64 / 1e6);
    assert_eq!(missing.load(Relaxed) + bad.load(Relaxed), 0);
    println!("VERIFIED {} infosets", n.load(Relaxed));
}
