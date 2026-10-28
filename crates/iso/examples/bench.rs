//! ns per `index_last` for the 7-card [2,5] indexer.
use cards::Card;
use std::time::Instant;

fn main() {
    let t = Instant::now();
    let ix = iso::HandIndexer::new(&[2, 5]);
    println!("build: {:?}, size {}", t.elapsed(), ix.size(1));
    // Deterministic xorshift deals, precomputed so only indexing is timed.
    let mut s = 0x9E3779B97F4A7C15u64;
    let n = 1 << 20;
    let mut deals: Vec<[Card; 7]> = Vec::with_capacity(n);
    for _ in 0..n {
        let mut d: Vec<Card> = (0..52).collect();
        let mut h = [0u8; 7];
        for (i, slot) in h.iter_mut().enumerate() {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let j = i + (s % (52 - i as u64)) as usize;
            d.swap(i, j);
            *slot = d[i];
        }
        deals.push(h);
    }
    let reps = 20;
    let t = Instant::now();
    let mut acc = 0u64;
    for _ in 0..reps {
        for h in &deals {
            acc = acc.wrapping_add(ix.index_last(h));
        }
    }
    let ns = t.elapsed().as_nanos() as f64 / (reps * n) as f64;
    println!("index_last [2,5]: {ns:.1} ns/index (checksum {acc})");
}
