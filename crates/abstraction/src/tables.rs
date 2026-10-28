//! Building and loading the blueprint card-bucket tables.
//!
//! For each postflop street we compute a feature for every suit-isomorphic (hole, board)
//! class (Waugh indices over rounds [2,3], [2,4], [2,5]), fit k-means on features of
//! uniformly sampled *raw* deals (so clusters are weighted by real frequency), then assign
//! every class to its nearest centroid. Tables are one `u16` bucket per class.
//!
//! - flop:  histogram of final river equity over all 1,081 turn+river runouts → EMD k-means
//! - turn:  histogram of river equity over the 46 rivers → EMD k-means
//! - river: OCHS, equity vs 8 opponent preflop-hand clusters → L2 k-means
//! The 8 opponent clusters are themselves EMD k-means over the 169 preflop classes'
//! river-equity distributions.

use crate::equity::{holes, river_cluster_equities, runout_histograms, NUM_HOLES};
use crate::kmeans::{to_cdf, KMeans, Metric};
use cards::{bit, mask_of, preflop_class, Card, Hand, Rng};
use iso::HandIndexer;
use rayon::prelude::*;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::Instant;

pub const OCHS_K: usize = 8;
pub const HIST_BINS: usize = 50;

pub struct BucketTables {
    pub buckets: [u16; 3],
    pub flop: Vec<u16>,
    pub turn: Vec<u16>,
    pub river: Vec<u16>,
    idx_flop: HandIndexer,
    idx_turn: HandIndexer,
    idx_river: HandIndexer,
}

impl BucketTables {
    /// Bucket of `hole` with a 0-, 3-, 4- or 5-card board. Preflop is lossless (169 classes).
    #[inline]
    pub fn bucket(&self, hole: [Card; 2], board: &[Card]) -> u32 {
        let mut cs = [0u8; 7];
        cs[..2].copy_from_slice(&hole);
        cs[2..2 + board.len()].copy_from_slice(board);
        let cs = &cs[..2 + board.len()];
        match board.len() {
            0 => preflop_class(hole[0], hole[1]) as u32,
            3 => self.flop[self.idx_flop.index_last(cs) as usize] as u32,
            4 => self.turn[self.idx_turn.index_last(cs) as usize] as u32,
            5 => self.river[self.idx_river.index_last(cs) as usize] as u32,
            n => panic!("bad board length {n}"),
        }
    }

    pub fn save(&self, dir: &Path) -> io::Result<()> {
        fs::create_dir_all(dir)?;
        for (name, t) in [("flop", &self.flop), ("turn", &self.turn), ("river", &self.river)] {
            let mut f = io::BufWriter::new(fs::File::create(dir.join(format!("{name}.u16")))?);
            for &b in t {
                f.write_all(&b.to_le_bytes())?;
            }
        }
        fs::write(dir.join("meta.txt"), format!("buckets {} {} {}\n", self.buckets[0], self.buckets[1], self.buckets[2]))
    }

    pub fn load(dir: &Path) -> io::Result<Self> {
        let read = |name: &str| -> io::Result<Vec<u16>> {
            let mut bytes = Vec::new();
            fs::File::open(dir.join(format!("{name}.u16")))?.read_to_end(&mut bytes)?;
            Ok(bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect())
        };
        let meta = fs::read_to_string(dir.join("meta.txt"))?;
        let nums: Vec<u16> = meta.split_whitespace().skip(1).map(|s| s.parse().unwrap()).collect();
        let t = BucketTables::empty([nums[0], nums[1], nums[2]]);
        let (flop, turn, river) = (read("flop")?, read("turn")?, read("river")?);
        assert_eq!(flop.len() as u64, t.idx_flop.size(1));
        assert_eq!(turn.len() as u64, t.idx_turn.size(1));
        assert_eq!(river.len() as u64, t.idx_river.size(1));
        Ok(BucketTables { flop, turn, river, ..t })
    }

    fn empty(buckets: [u16; 3]) -> Self {
        BucketTables {
            buckets,
            flop: vec![],
            turn: vec![],
            river: vec![],
            idx_flop: HandIndexer::new(&[2, 3]),
            idx_turn: HandIndexer::new(&[2, 4]),
            idx_river: HandIndexer::new(&[2, 5]),
        }
    }
}

/// Enumerate canonical boards of `n` cards (one representative per suit-isomorphism class).
fn canonical_boards(n: u8) -> Vec<Hand> {
    let idx = HandIndexer::new(&[n]);
    (0..idx.size(0))
        .map(|i| {
            let mut out = vec![0u8; n as usize];
            assert!(idx.unindex(0, i, &mut out));
            mask_of(&out)
        })
        .collect()
}

fn board_cards(m: Hand) -> Vec<Card> {
    cards::cards_in(m).collect()
}

/// Opponent clusters for OCHS: EMD k-means over the 169 preflop classes' river-equity
/// histograms (estimated from `samples` random boards). Returns the cluster of every hole.
pub fn opponent_clusters(samples: usize, seed: u64) -> Vec<u8> {
    let mut rng = Rng::new(seed);
    let boards: Vec<Hand> = (0..samples)
        .map(|_| {
            let mut b = 0;
            for _ in 0..5 {
                b |= bit(rng.draw(b));
            }
            b
        })
        .collect();
    let hist = boards
        .par_chunks(1024)
        .map(|chunk| {
            let mut h = vec![[0f32; HIST_BINS]; 169];
            let mut eq = [0f32; NUM_HOLES];
            for &b in chunk {
                crate::equity::river_equities(b, &mut eq);
                for (i, &hole) in holes().iter().enumerate() {
                    if !eq[i].is_nan() {
                        let bin = ((eq[i] * HIST_BINS as f32) as usize).min(HIST_BINS - 1);
                        h[preflop_class(hole[0], hole[1])][bin] += 1.0;
                    }
                }
            }
            h
        })
        .reduce(|| vec![[0f32; HIST_BINS]; 169], |mut a, b| {
            a.iter_mut().zip(b).for_each(|(x, y)| x.iter_mut().zip(y).for_each(|(p, q)| *p += q));
            a
        });
    let mut pts = Vec::with_capacity(169 * HIST_BINS);
    let mut w = Vec::with_capacity(169);
    for (c, h) in hist.iter().enumerate() {
        let mut v = h.to_vec();
        to_cdf(&mut v);
        pts.extend(v);
        w.push(if c < 13 { 6.0 } else if c < 91 { 4.0 } else { 12.0 });
    }
    let (mut km, assign) = KMeans::fit(&pts, Some(&w), HIST_BINS, OCHS_K, Metric::Emd1d, 100, seed);
    let remap = km.sort_by_strength();
    holes().iter().map(|h| remap[assign[preflop_class(h[0], h[1])] as usize] as u8).collect()
}

/// Feature rows for every class of `indexer` (rounds [2, board_len]), computed per canonical
/// board. `feat(board) -> Vec<per-hole feature row>` returns `dim` values per hole.
fn features_by_index(indexer: &HandIndexer, board_len: u8, dim: usize, feat: impl Fn(Hand) -> Vec<Vec<u16>> + Sync) -> Vec<u16> {
    let size = indexer.size(1) as usize;
    let mut out = vec![0u16; size * dim];
    let mut filled = vec![false; size];
    let boards = canonical_boards(board_len);
    let t0 = Instant::now();
    for (ci, chunk) in boards.chunks(2048).enumerate() {
        let rows: Vec<(usize, Vec<u16>)> = chunk
            .par_iter()
            .flat_map_iter(|&b| {
                let f = feat(b);
                let bc = board_cards(b);
                let mut res = Vec::with_capacity(1081);
                for (i, h) in holes().iter().enumerate() {
                    if (bit(h[0]) | bit(h[1])) & b != 0 {
                        continue;
                    }
                    let mut cs = vec![h[0], h[1]];
                    cs.extend(&bc);
                    res.push((indexer.index_last(&cs) as usize, f[i].clone()));
                }
                res
            })
            .collect();
        for (idx, row) in rows {
            out[idx * dim..(idx + 1) * dim].copy_from_slice(&row);
            filled[idx] = true;
        }
        if ci % 10 == 0 {
            eprintln!("  features: {}/{} boards, {:.0}s", (ci + 1) * 2048, boards.len(), t0.elapsed().as_secs_f64());
        }
    }
    let missing = filled.iter().filter(|&&f| !f).count();
    assert_eq!(missing, 0, "{missing} classes never filled");
    out
}

/// Fit k-means on features of `sample` uniformly random raw deals, then assign every class.
fn cluster(indexer: &HandIndexer, board_len: usize, feats: &[u16], dim: usize, k: usize, metric: Metric, sample: usize, seed: u64) -> Vec<u16> {
    let to_point = |row: &[u16]| -> Vec<f32> {
        let mut v: Vec<f32> = row.iter().map(|&x| x as f32).collect();
        match metric {
            Metric::Emd1d => to_cdf(&mut v),
            Metric::L2 => v.iter_mut().for_each(|x| *x /= 65535.0),
        }
        v
    };
    let mut rng = Rng::new(seed);
    let mut pts = Vec::with_capacity(sample * dim);
    for _ in 0..sample {
        let mut dead = 0;
        let mut cs = Vec::with_capacity(2 + board_len);
        for _ in 0..2 + board_len {
            let c = rng.draw(dead);
            dead |= bit(c);
            cs.push(c);
        }
        let idx = indexer.index_last(&cs) as usize;
        pts.extend(to_point(&feats[idx * dim..(idx + 1) * dim]));
    }
    let (mut km, _) = KMeans::fit(&pts, None, dim, k, metric, 40, seed);
    // Centroids are re-ordered by strength, so `nearest` already returns sorted bucket ids.
    km.sort_by_strength();
    let n = feats.len() / dim;
    (0..n)
        .into_par_iter()
        .map(|i| km.nearest(&to_point(&feats[i * dim..(i + 1) * dim])).0 as u16)
        .collect()
}


pub fn build(buckets: [u16; 3], sample: usize, seed: u64) -> BucketTables {
    let mut t = BucketTables::empty(buckets);
    let t0 = Instant::now();

    eprintln!("[1/4] opponent clusters for OCHS");
    let opp = opponent_clusters(200_000, seed);

    eprintln!("[2/4] river OCHS features ({} classes)", t.idx_river.size(1));
    let river_feats = features_by_index(&t.idx_river, 5, OCHS_K, |b| {
        let mut eq = vec![[0f32; OCHS_K]; NUM_HOLES];
        river_cluster_equities::<OCHS_K>(b, &opp, &mut eq);
        eq.iter().map(|r| r.iter().map(|&x| (x * 65535.0).round() as u16).collect()).collect()
    });
    t.river = cluster(&t.idx_river, 5, &river_feats, OCHS_K, buckets[2] as usize, Metric::L2, sample, seed ^ 3);
    drop(river_feats);
    eprintln!("  river done {:.0}s", t0.elapsed().as_secs_f64());

    eprintln!("[3/4] turn histogram features ({} classes)", t.idx_turn.size(1));
    let turn_feats = features_by_index(&t.idx_turn, 4, HIST_BINS, |b| runout_histograms(b, HIST_BINS));
    t.turn = cluster(&t.idx_turn, 4, &turn_feats, HIST_BINS, buckets[1] as usize, Metric::Emd1d, sample, seed ^ 2);
    drop(turn_feats);
    eprintln!("  turn done {:.0}s", t0.elapsed().as_secs_f64());

    eprintln!("[4/4] flop histogram features ({} classes)", t.idx_flop.size(1));
    let flop_feats = features_by_index(&t.idx_flop, 3, HIST_BINS, |b| runout_histograms(b, HIST_BINS));
    t.flop = cluster(&t.idx_flop, 3, &flop_feats, HIST_BINS, buckets[0] as usize, Metric::Emd1d, sample, seed ^ 1);
    eprintln!("  flop done {:.0}s", t0.elapsed().as_secs_f64());
    t
}
