//! Scale test of the streaming compactor: synthesise a big checkpoint (random keys, 2-5
//! actions, some first-round infosets with φ), save it with `io::save` (the real writer, so the
//! on-disk record order is the real one), compact it with the default options and verify a
//! random sample of infosets against the original store.
//!
//! cargo run --release -p blueprint --example compact_scale -- N_INFOSETS [THREADS] [DIR]
use blueprint::compact::{compact_checkpoint, Compact, CompactOptions};
use blueprint::store::{infoset_key, Store};
use rayon::prelude::*;
use std::path::PathBuf;
use std::time::Instant;

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn spec(i: u64) -> (u64, u32, usize, bool) {
    let h = mix(i);
    (h, (mix(i ^ 0xABCD) % 200) as u32, 2 + (mix(i ^ 0x77) % 4) as usize, mix(i ^ 0x99) % 40 == 0)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: u64 = a.get(1).map_or(150_000_000, |s| s.parse().unwrap());
    let threads: usize = a.get(2).map_or(8, |s| s.parse().unwrap());
    let dir = PathBuf::from(a.get(3).cloned().unwrap_or("data/compact-scale".into()));
    std::fs::create_dir_all(&dir).unwrap();
    rayon::ThreadPoolBuilder::new().num_threads(threads.max(2)).build_global().unwrap();
    let t = Instant::now();
    let st = Store::with_index((n * 7) as usize, n as usize);
    (0..n).into_par_iter().for_each(|i| {
        let (h, b, na, avg) = spec(i);
        let s = st.infoset(h, b, na, 200, avg);
        for k in 0..na {
            st.add_regret(s, k, ((mix(i ^ (k as u64 + 1) * 31) % 1500) as f32) - 500.0);
            if avg {
                for _ in 0..(mix(i ^ (k as u64 + 9)) % 4) {
                    st.bump_avg(s, k);
                }
            }
        }
    });
    eprintln!("built {} infosets in {:.0}s", st.infosets(), t.elapsed().as_secs_f64());
    let ck = dir.join("ck.bin");
    let t = Instant::now();
    blueprint::io::save(&st, &ck).unwrap();
    eprintln!("saved {:.2} GB in {:.0}s", std::fs::metadata(&ck).unwrap().len() as f64 / 1e9, t.elapsed().as_secs_f64());
    let out = dir.join("out.plcp");
    let mut opt = CompactOptions { threads, ..Default::default() };
    if let Ok(g) = std::env::var("GROUP_SLOTS") {
        opt.group_slots = g.parse().unwrap();
    }
    let t = Instant::now();
    let stats = compact_checkpoint(&ck, None, &out, &opt, &|m| eprintln!("{m}")).unwrap();
    let secs = t.elapsed().as_secs_f64();
    eprintln!("compact: {secs:.0}s = {:.2} M infosets/s, {:.2} B/infoset", n as f64 / secs / 1e6, stats.bytes_out as f64 / n as f64);
    let c = Compact::open(&out).unwrap();
    assert_eq!(c.len() as u64, stats.infosets_out);
    // Verify a random sample against the store (and sortedness at the sampled keys' neighbours).
    let samples = 2_000_000u64.min(n);
    let (bad, worst) = (0..samples)
        .into_par_iter()
        .map(|j| {
            let i = mix(j ^ 0x5151) % n;
            let (h, b, na, avg) = spec(i);
            let slot = st.lookup(h, b).unwrap();
            let mut p = vec![0f32; na];
            if avg {
                st.average(slot, &mut p);
            } else {
                st.strategy(slot, &mut p);
            }
            let q = c.lookup(h, b).expect("sampled infoset present");
            let e = (0..na).map(|k| (q[k] as f32 / 255.0 - p[k]).abs()).fold(0f32, f32::max);
            (u64::from(q.len() != na || e > 1.0 / 255.0 + 1e-4), e)
        })
        .reduce(|| (0, 0f32), |x, y| (x.0 + y.0, x.1.max(y.1)));
    eprintln!("verified {samples} samples: {bad} bad, worst error {worst:.5}");
    assert_eq!(bad, 0);
    let mut prev = c.key_at(0);
    for i in (1..c.len()).step_by(997) {
        let k = c.key_at(i);
        assert!(k > prev);
        prev = k;
    }
    // Lookup speed (random present keys), single thread.
    let t = Instant::now();
    let mut acc = 0u64;
    for j in 0..2_000_000u64 {
        let (h, b, _, _) = spec(mix(j) % n);
        acc += c.lookup(h, b).map_or(0, |q| q[0] as u64);
    }
    eprintln!("lookup: {:.0} ns/op (1 thread, {} sink)", t.elapsed().as_secs_f64() * 1e9 / 2e6, acc);
    let _ = infoset_key(0, 0);
    if std::env::var("KEEP").is_err() {
        let _ = std::fs::remove_file(&ck);
        let _ = std::fs::remove_file(&out);
    }
}
