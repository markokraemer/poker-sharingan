//! Build blueprint bucket tables: `build_buckets <out_dir> [flop turn river] [sample]`.
use abstraction::tables::build;
use std::path::PathBuf;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let out = PathBuf::from(a.get(1).cloned().unwrap_or("data/buckets-200".into()));
    let n = |i: usize, d: u16| a.get(i).map(|s| s.parse().unwrap()).unwrap_or(d);
    let buckets = [n(2, 200), n(3, 200), n(4, 200)];
    let sample = a.get(5).map(|s| s.parse().unwrap()).unwrap_or(2_000_000usize);
    let t = build(buckets, sample, 2019);
    t.save(&out).expect("save tables");
    eprintln!("saved to {}", out.display());
}
