//! Checkpoint save/load throughput on a synthetic store: `save_bench N_INFOSETS PATH`.
use blueprint::{io, store::Store};
use rayon::prelude::*;
use std::time::Instant;

fn main() {
    let n: u64 = std::env::args().nth(1).map(|s| s.parse().unwrap()).unwrap_or(200_000_000);
    let path = std::path::PathBuf::from(std::env::args().nth(2).unwrap_or("/tmp/save_bench.bin".into()));
    let st = Store::with_index(n as usize * 4, n as usize);
    let t = Instant::now();
    (0..n).into_par_iter().for_each(|i| {
        let s = st.infoset(i.wrapping_mul(0x9E37_79B9_7F4A_7C15), (i % 200) as u32, 3, 200, false);
        st.add_regret(s, 0, i as f32);
    });
    println!("fill {} infosets: {:.1}s", st.infosets(), t.elapsed().as_secs_f64());
    let t = Instant::now();
    io::save(&st, &path).unwrap();
    let bytes = std::fs::metadata(&path).unwrap().len();
    let secs = t.elapsed().as_secs_f64();
    println!("save {:.1} GB in {:.1}s = {:.0} MB/s", bytes as f64 / 1e9, secs, bytes as f64 / 1e6 / secs);
    let t = Instant::now();
    let ld = io::load(&path, 0, 0).unwrap();
    println!("load {} infosets in {:.1}s", ld.infosets(), t.elapsed().as_secs_f64());
    std::fs::remove_file(&path).ok();
}
