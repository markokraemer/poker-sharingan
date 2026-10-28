//! Sanity: bucket ids should increase with hand strength; report how much of the equity
//! variance the buckets explain (R²).
use abstraction::tables::BucketTables;
use cards::*;

fn main() {
    let dir = std::env::args().nth(1).unwrap_or("data/buckets-200".into());
    let t = BucketTables::load(std::path::Path::new(&dir)).unwrap();
    let k = t.buckets[0] as usize;
    let mut rng = Rng::new(11);
    for (street, n) in [("flop", 3usize), ("turn", 4), ("river", 5)] {
        let (mut sum, mut sq, mut cnt) = (vec![0f64; k], vec![0f64; k], vec![0f64; k]);
        for _ in 0..20_000 {
            let mut dead = 0;
            let mut cs = vec![];
            for _ in 0..2 + n {
                let c = rng.draw(dead);
                dead |= bit(c);
                cs.push(c);
            }
            let hole = [cs[0], cs[1]];
            let b = t.bucket(hole, &cs[2..]) as usize;
            let mut eq = 0.0;
            for _ in 0..64 {
                let mut d = dead;
                let mut board = mask_of(&cs[2..]);
                for _ in n..5 {
                    let c = rng.draw(d);
                    d |= bit(c);
                    board |= bit(c);
                }
                let o1 = rng.draw(d);
                d |= bit(o1);
                let o2 = rng.draw(d);
                let me = evaluate(board | bit(hole[0]) | bit(hole[1]));
                let op = evaluate(board | bit(o1) | bit(o2));
                eq += if me > op { 1.0 } else if me == op { 0.5 } else { 0.0 };
            }
            eq /= 64.0;
            sum[b] += eq;
            sq[b] += eq * eq;
            cnt[b] += 1.0;
        }
        let means: Vec<f64> = (0..k).filter(|&b| cnt[b] > 0.0).map(|b| sum[b] / cnt[b]).collect();
        let inv = means.windows(2).filter(|w| w[1] + 0.05 < w[0]).count();
        let n_all: f64 = cnt.iter().sum();
        let within: f64 = (0..k).filter(|&b| cnt[b] > 0.0).map(|b| sq[b] - sum[b] * sum[b] / cnt[b]).sum::<f64>() / n_all;
        let m = sum.iter().sum::<f64>() / n_all;
        let total = sq.iter().sum::<f64>() / n_all - m * m;
        println!(
            "{street}: buckets hit {}, weakest/strongest bucket equity {:.2}/{:.2}, inversions>0.05: {inv}, R^2 of equity {:.3}",
            means.len(),
            means[0],
            means[means.len() - 1],
            1.0 - within / total
        );
    }
}
