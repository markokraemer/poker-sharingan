//! The real-time-search abstraction of Pluribus (supplementary: "500 buckets per round ...
//! determined separately for each flop using an algorithm that considers the future potential
//! of each poker hand ... potential-aware abstraction combined with clustering based on
//! earth-mover distance").
//!
//! For each of the 1,755 strategically distinct flops, *within that flop only*:
//!
//! * **river**: every (hole, turn, river) situation -> exact OCHS features (equity against 8
//!   opponent preflop-hand clusters, `river_cluster_equities`) -> L2 k-means -> `k_river` buckets;
//! * **turn**: every (hole, turn) situation -> histogram over this flop's *river buckets* of the
//!   river card (46 equally likely rivers) -> k-means under an earth-mover distance whose ground
//!   distance between two river buckets is the distance between their centroids -> `k_turn` buckets.
//!
//! The EMD is approximated by a tree embedding (see [`BucketTree`]): river buckets are grouped
//! hierarchically by k-means on their centroids, and the transport cost on that tree is an L1
//! distance between sparse vectors, so k-means with mean centroids is cheap. Bucket ids are
//! ordered by strength. Tables are suit-canonical per flop; [`FlopView`] maps arbitrary suits.

use crate::equity::{hole_mask, holes, river_cluster_equities, NUM_HOLES};
use crate::kmeans::{KMeans, Metric, QUIET};
use crate::tables::{opponent_clusters, OCHS_K};
use cards::{bit, hole_index, mask_of, Card, Rng};
use rayon::prelude::*;
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

pub const NUM_FLOPS: usize = 1755;
/// Marks (hole, board) combinations that collide with each other.
pub const NO_BUCKET: u16 = u16::MAX;
const TURN_LEN: usize = NUM_HOLES * 52;
const RIVER_LEN: usize = NUM_HOLES * NUM_HOLES;

// ---------------------------------------------------------------------------------------------
// Flop canonicalisation

fn suit_perms() -> Vec<[u8; 4]> {
    let mut v = Vec::with_capacity(24);
    for a in 0..4u8 {
        for b in 0..4u8 {
            for c in 0..4u8 {
                for d in 0..4u8 {
                    let mut seen = 0u8;
                    for x in [a, b, c, d] {
                        seen |= 1 << x;
                    }
                    if seen == 0b1111 {
                        v.push([a, b, c, d]);
                    }
                }
            }
        }
    }
    v
}

#[inline(always)]
fn map_card(c: Card, perm: &[u8; 4]) -> Card {
    (c & !3) | perm[(c & 3) as usize]
}

fn mapped_sorted(f: [Card; 3], perm: &[u8; 4]) -> [Card; 3] {
    let mut m = [map_card(f[0], perm), map_card(f[1], perm), map_card(f[2], perm)];
    m.sort_unstable();
    m
}

/// The 1,755 suit-isomorphism classes of flops.
pub struct FlopCanon {
    perms: Vec<[u8; 4]>,
    flops: Vec<[Card; 3]>,
    ids: HashMap<[Card; 3], u16>,
}

impl FlopCanon {
    pub fn new() -> Self {
        let perms = suit_perms();
        let mut set = BTreeSet::new();
        for a in 0..52u8 {
            for b in a + 1..52 {
                for c in b + 1..52 {
                    set.insert(perms.iter().map(|p| mapped_sorted([a, b, c], p)).min().unwrap());
                }
            }
        }
        let flops: Vec<[Card; 3]> = set.into_iter().collect();
        let ids = flops.iter().enumerate().map(|(i, f)| (*f, i as u16)).collect();
        FlopCanon { perms, flops, ids }
    }

    pub fn len(&self) -> usize {
        self.flops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.flops.is_empty()
    }

    pub fn flop(&self, id: u16) -> [Card; 3] {
        self.flops[id as usize]
    }

    /// Canonical flop id of `flop` and a suit permutation `p` with `p(flop) == canonical flop`.
    pub fn canonicalize(&self, flop: [Card; 3]) -> (u16, [u8; 4]) {
        let mut best = (mapped_sorted(flop, &self.perms[0]), self.perms[0]);
        for p in &self.perms[1..] {
            let m = mapped_sorted(flop, p);
            if m < best.0 {
                best = (m, *p);
            }
        }
        (self.ids[&best.0], best.1)
    }
}

impl Default for FlopCanon {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------------------------
// Build parameters

#[derive(Clone, Debug)]
pub struct BuildParams {
    pub k_turn: usize,
    pub k_river: usize,
    /// Situations sampled to fit the river / turn centroids (all situations are then assigned).
    pub river_fit_sample: usize,
    pub turn_fit_sample: usize,
    pub river_iters: usize,
    pub turn_iters: usize,
    /// Branching factor of the river-bucket tree used by the approximate EMD.
    pub fanout: usize,
    /// Number of hierarchies averaged in the tree-embedded EMD.
    pub trees: usize,
    /// Random boards for the opponent clusters behind the OCHS features.
    pub opp_samples: usize,
    pub seed: u64,
}

impl Default for BuildParams {
    /// The paper's search abstraction: 500 turn + 500 river buckets per flop.
    fn default() -> Self {
        BuildParams {
            k_turn: 500,
            k_river: 500,
            river_fit_sample: 60_000,
            turn_fit_sample: 20_000,
            river_iters: 20,
            turn_iters: 14,
            fanout: 8,
            trees: 4,
            opp_samples: 200_000,
            seed: 2019,
        }
    }
}

impl BuildParams {
    /// Cheap settings for tests.
    pub fn tiny() -> Self {
        BuildParams {
            k_turn: 10,
            k_river: 12,
            river_fit_sample: 4_000,
            turn_fit_sample: 1_500,
            river_iters: 6,
            turn_iters: 6,
            fanout: 4,
            trees: 2,
            opp_samples: 3_000,
            seed: 7,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct BuildStats {
    pub river_features_s: f64,
    pub river_cluster_s: f64,
    pub turn_features_s: f64,
    pub turn_cluster_s: f64,
    /// Pearson correlation between tree-EMD ground distance and true centroid distance.
    pub tree_corr: f32,
}

impl BuildStats {
    pub fn total_s(&self) -> f64 {
        self.river_features_s + self.river_cluster_s + self.turn_features_s + self.turn_cluster_s
    }
}

// ---------------------------------------------------------------------------------------------
// Tree-embedded earth mover's distance over river buckets

/// Ensemble of `T` three-level hierarchies over the river buckets (root -> L1 groups -> L2
/// groups -> buckets), each built by k-means on the bucket centroids with a different seed and
/// fanout. The earth mover's distance between two histograms over buckets, with a tree metric as
/// ground distance, is `sum_edges w_e * |mass_e(p) - mass_e(q)|`: an L1 distance between
/// (sparse) embedded vectors. Edge weights are half the mean centroid distance between
/// siblings of that level; the `T` trees are averaged (concatenated embeddings scaled by 1/T,
/// which lowers the distortion of any single tree) and the whole is rescaled so the mean tree
/// distance between two buckets equals the mean centroid distance (Indyk-Thaper style
/// embedding; the Pearson correlation with the true centroid distance is reported in
/// `BuildStats`).
pub struct BucketTree {
    pub k: usize,
    pub trees: usize,
    /// Total number of embedding nodes.
    pub nodes: usize,
    /// Global node ids (L1, L2, leaf) of bucket `b` in tree `t` at `path[b * trees + t]`.
    pub path: Vec<[u16; 3]>,
    /// Edge weight into every node.
    pub weight: Vec<f32>,
    /// First leaf node of tree 0 (leaf `b` = `leaf0 + b`).
    pub leaf0: usize,
    /// Exact 1-D EMD component along bucket strength: bucket -> bin (consecutive ids), the
    /// bin-to-bin ground distance is the difference of the bins' mean strength; `gap[j]` is the
    /// (scaled) distance between bins `j` and `j+1`, embedded as `gap[j] * CDF_j` at node
    /// `cdf0 + j`.
    pub bin_of: Vec<u8>,
    pub gap: Vec<f32>,
    pub cdf0: usize,
    pub corr: f32,
}

fn l2(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f32>().sqrt()
}

fn mean_vec(rows: &[&[f32]]) -> Vec<f32> {
    let dim = rows[0].len();
    let mut m = vec![0f32; dim];
    for r in rows {
        m.iter_mut().zip(*r).for_each(|(a, b)| *a += b);
    }
    m.iter_mut().for_each(|a| *a /= rows.len() as f32);
    m
}

/// Group `points` (row-major) into about `g` clusters; returns dense group ids.
fn group(points: &[f32], dim: usize, g: usize, seed: u64) -> (Vec<usize>, usize) {
    let n = points.len() / dim;
    let assign: Vec<usize> = if g <= 1 || n <= g {
        if g <= 1 {
            vec![0; n]
        } else {
            (0..n).collect()
        }
    } else {
        KMeans::fit(points, None, dim, g, Metric::L2, 15, seed).1.into_iter().map(|a| a as usize).collect()
    };
    let mut remap = vec![usize::MAX; assign.iter().max().map_or(0, |m| m + 1)];
    let mut next = 0;
    let out: Vec<usize> = assign
        .iter()
        .map(|&a| {
            if remap[a] == usize::MAX {
                remap[a] = next;
                next += 1;
            }
            remap[a]
        })
        .collect();
    (out, next)
}

/// One hierarchy: (local node ids per bucket, g1, g2, raw level weights).
fn single_tree(centroids: &[f32], dim: usize, fanout: usize, seed: u64) -> (Vec<[usize; 3]>, usize, usize, [f32; 3]) {
    let k = centroids.len() / dim;
    let row = |i: usize| &centroids[i * dim..(i + 1) * dim];
    let g2t = ((k as f32 / fanout as f32).round() as usize).clamp(1, k);
    let g1t = ((g2t as f32 / fanout as f32).round() as usize).clamp(1, g2t);
    let (a2, g2) = group(centroids, dim, g2t, seed);
    let mut mean2 = Vec::with_capacity(g2 * dim);
    for g in 0..g2 {
        let rows: Vec<&[f32]> = (0..k).filter(|&i| a2[i] == g).map(row).collect();
        mean2.extend(mean_vec(&rows));
    }
    let (a1, g1) = group(&mean2, dim, g1t, seed ^ 1);
    let mut sum = [0f64; 3];
    let mut cnt = [0f64; 3];
    let mut pair = |lvl: usize, a: &[f32], b: &[f32]| {
        sum[lvl] += l2(a, b) as f64;
        cnt[lvl] += 1.0;
    };
    let m2 = |g: usize| &mean2[g * dim..(g + 1) * dim];
    let m1: Vec<Vec<f32>> = (0..g1)
        .map(|g| {
            let rows: Vec<&[f32]> = (0..k).filter(|&i| a1[a2[i]] == g).map(row).collect();
            mean_vec(&rows)
        })
        .collect();
    for x in 0..g1 {
        for y in x + 1..g1 {
            pair(0, &m1[x], &m1[y]);
        }
    }
    for x in 0..g2 {
        for y in x + 1..g2 {
            if a1[x] == a1[y] {
                pair(1, m2(x), m2(y));
            }
        }
    }
    for x in 0..k {
        for y in x + 1..k {
            if a2[x] == a2[y] {
                pair(2, row(x), row(y));
            }
        }
    }
    let mut lw = [0f32; 3];
    for l in 0..3 {
        if cnt[l] > 0.0 {
            lw[l] = (0.5 * sum[l] / cnt[l]) as f32;
        }
    }
    let path = (0..k).map(|i| [a1[a2[i]], g1 + a2[i], g1 + g2 + i]).collect();
    (path, g1, g2, lw)
}

impl BucketTree {
    /// `centroids`: `k` row-major vectors of length `dim`; `trees` hierarchies around `fanout`.
    pub fn build(centroids: &[f32], dim: usize, fanout: usize, trees: usize, seed: u64) -> Self {
        let k = centroids.len() / dim;
        let row = |i: usize| &centroids[i * dim..(i + 1) * dim];
        let trees = trees.max(1);
        let mut path = vec![[0u16; 3]; k * trees];
        let mut raw_w: Vec<f32> = vec![];
        let mut leaf0 = 0;
        for t in 0..trees {
            let f = (fanout as f32 * [1.0, 0.75, 1.4, 0.6, 1.2, 0.9][t % 6]).round().max(2.0) as usize;
            let (p, g1, g2, lw) = single_tree(centroids, dim, f, seed ^ (t as u64 + 1).wrapping_mul(0xD1B5_4A32_D192_ED03));
            let off = raw_w.len();
            if t == 0 {
                leaf0 = off + g1 + g2;
            }
            for n in 0..g1 + g2 + k {
                raw_w.push(lw[if n < g1 { 0 } else if n < g1 + g2 { 1 } else { 2 }]);
            }
            for b in 0..k {
                path[b * trees + t] = [(off + p[b][0]) as u16, (off + p[b][1]) as u16, (off + p[b][2]) as u16];
            }
        }
        let tree_nodes = raw_w.len();
        // 1-D strength component: bins of consecutive bucket ids (ids are sorted by strength).
        let m = k.min(64);
        let bin_of: Vec<u8> = (0..k).map(|b| (b * m / k) as u8).collect();
        let strength = |b: usize| row(b).iter().sum::<f32>() / dim as f32;
        let mut bin_mean = vec![0f32; m];
        let mut bin_cnt = vec![0f32; m];
        for b in 0..k {
            bin_mean[bin_of[b] as usize] += strength(b);
            bin_cnt[bin_of[b] as usize] += 1.0;
        }
        for j in 0..m {
            bin_mean[j] /= bin_cnt[j].max(1.0);
        }
        let gap: Vec<f32> = (0..m.saturating_sub(1)).map(|j| (bin_mean[j + 1] - bin_mean[j]).max(0.0)).collect();
        let nodes = tree_nodes + gap.len();
        let mut t = BucketTree {
            k,
            trees,
            nodes,
            path,
            weight: raw_w.iter().map(|w| w.max(1e-6) / trees as f32).collect(),
            leaf0,
            bin_of,
            gap,
            cdf0: tree_nodes,
            corr: 0.0,
        };
        // Calibrate: each component alone is scaled so its mean distance equals the mean
        // centroid distance, and the two are then averaged.
        let (mut st, mut s1, mut sl) = (0f64, 0f64, 0f64);
        for x in 0..k {
            for y in x + 1..k {
                st += t.distance_tree(x, y) as f64;
                s1 += t.distance_1d(x, y) as f64;
                sl += l2(row(x), row(y)) as f64;
            }
        }
        if st > 0.0 {
            let sc = (0.5 * sl / st) as f32;
            t.weight.iter_mut().for_each(|w| *w *= sc);
        }
        if s1 > 0.0 {
            let sc = (0.5 * sl / s1) as f32;
            t.gap.iter_mut().for_each(|g| *g *= sc);
        }
        let (mut sd, mut sl, mut sdd, mut sll, mut sdl, mut n) = (0f64, 0f64, 0f64, 0f64, 0f64, 0f64);
        for x in 0..k {
            for y in x + 1..k {
                let (d_t, d_l) = (t.distance(x, y) as f64, l2(row(x), row(y)) as f64);
                sd += d_t;
                sl += d_l;
                sdd += d_t * d_t;
                sll += d_l * d_l;
                sdl += d_t * d_l;
                n += 1.0;
            }
        }
        if n > 0.0 {
            let (mt, ml) = (sd / n, sl / n);
            let (vt, vl) = (sdd / n - mt * mt, sll / n - ml * ml);
            if vt > 0.0 && vl > 0.0 {
                t.corr = ((sdl / n - mt * ml) / (vt * vl).sqrt()) as f32;
            }
        }
        t
    }

    pub fn n_nodes(&self) -> usize {
        self.nodes
    }

    /// Tree part of the ground distance (mean over the ensemble).
    fn distance_tree(&self, a: usize, b: usize) -> f32 {
        if a == b {
            return 0.0;
        }
        let mut d = 0.0;
        for t in 0..self.trees {
            let (pa, pb) = (&self.path[a * self.trees + t], &self.path[b * self.trees + t]);
            for l in (0..3).rev() {
                if pa[l] == pb[l] {
                    break;
                }
                d += self.weight[pa[l] as usize] + self.weight[pb[l] as usize];
            }
        }
        d
    }

    /// 1-D strength part of the ground distance.
    fn distance_1d(&self, a: usize, b: usize) -> f32 {
        let (ba, bb) = (self.bin_of[a] as usize, self.bin_of[b] as usize);
        let (lo, hi) = (ba.min(bb), ba.max(bb));
        self.gap[lo..hi].iter().sum()
    }

    /// Ground distance between two river buckets.
    pub fn distance(&self, a: usize, b: usize) -> f32 {
        self.distance_tree(a, b) + self.distance_1d(a, b)
    }

    /// Embedded sparse vector of a uniform histogram over `buckets` (sorted by node id, merged).
    pub fn embed(&self, buckets: impl Iterator<Item = u16>, out: &mut Vec<(u16, f32)>) {
        out.clear();
        let mut n = 0f32;
        let mut cnt = [0f32; 64];
        for b in buckets {
            n += 1.0;
            cnt[self.bin_of[b as usize] as usize] += 1.0;
            for p in &self.path[b as usize * self.trees..(b as usize + 1) * self.trees] {
                for &node in p {
                    out.push((node, self.weight[node as usize]));
                }
            }
        }
        let mut cum = 0f32;
        for (j, &g) in self.gap.iter().enumerate() {
            cum += cnt[j];
            if cum > 0.0 && g > 0.0 {
                out.push(((self.cdf0 + j) as u16, g * cum));
            }
        }
        for e in out.iter_mut() {
            e.1 /= n;
        }
        out.sort_unstable_by_key(|e| e.0);
        let mut w = 0;
        for r in 0..out.len() {
            if w > 0 && out[w - 1].0 == out[r].0 {
                out[w - 1].1 += out[r].1;
            } else {
                out[w] = out[r];
                w += 1;
            }
        }
        out.truncate(w);
    }
}

// ---------------------------------------------------------------------------------------------
// Sparse L1 k-means (centroids = means; used with tree-embedded histograms)

pub struct SparseSet {
    offs: Vec<u32>,
    idx: Vec<u16>,
    val: Vec<f32>,
}

impl SparseSet {
    fn new() -> Self {
        SparseSet { offs: vec![0], idx: vec![], val: vec![] }
    }
    fn push(&mut self, row: &[(u16, f32)]) {
        for &(j, v) in row {
            self.idx.push(j);
            self.val.push(v);
        }
        self.offs.push(self.idx.len() as u32);
    }
    pub fn len(&self) -> usize {
        self.offs.len() - 1
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    #[inline]
    fn row(&self, i: usize) -> (&[u16], &[f32]) {
        let (a, b) = (self.offs[i] as usize, self.offs[i + 1] as usize);
        (&self.idx[a..b], &self.val[a..b])
    }
}

/// L1 distance between a sparse non-negative point and a dense non-negative centroid with
/// precomputed L1 norm `cl1`.
#[inline]
fn sdist(ri: &[u16], rv: &[f32], c: &[f32], cl1: f32) -> f32 {
    let mut d = cl1;
    for (&j, &v) in ri.iter().zip(rv) {
        let cj = c[j as usize];
        d += (v - cj).abs() - cj;
    }
    d
}

fn snearest(set: &SparseSet, i: usize, cents: &[f32], l1: &[f32], dim: usize) -> (usize, f32) {
    let (ri, rv) = set.row(i);
    let mut best = (0, f32::INFINITY);
    for (c, cent) in cents.chunks_exact(dim).enumerate() {
        let d = sdist(ri, rv, cent, l1[c]);
        if d < best.1 {
            best = (c, d);
        }
    }
    best
}

fn l1_norms(cents: &[f32], dim: usize) -> Vec<f32> {
    cents.chunks_exact(dim).map(|c| c.iter().sum()).collect()
}

/// Fit `k` centroids on the points `fit` (indices into `set`).
pub fn sparse_kmeans(set: &SparseSet, fit: &[usize], dim: usize, k: usize, iters: usize, seed: u64) -> Vec<f32> {
    let nfit = fit.len();
    assert!(nfit >= k && k >= 1);
    let mut rng = Rng::new(seed);
    let densify = |i: usize, out: &mut [f32]| {
        out.fill(0.0);
        let (ri, rv) = set.row(i);
        for (&j, &v) in ri.iter().zip(rv) {
            out[j as usize] = v;
        }
    };
    let mut cents = vec![0f32; k * dim];
    densify(fit[rng.below(nfit as u32) as usize], &mut cents[..dim]);
    let l1c = cents[..dim].iter().sum::<f32>();
    let mut d: Vec<f32> = {
        let c0 = &cents[..dim];
        fit.par_iter()
            .map(|&i| {
                let (ri, rv) = set.row(i);
                sdist(ri, rv, c0, l1c)
            })
            .collect()
    };
    for c in 1..k {
        let probs: Vec<f32> = d.iter().map(|x| x * x).collect();
        let pick = fit[rng.sample_weighted(&probs)];
        densify(pick, &mut cents[c * dim..(c + 1) * dim]);
        let cc = &cents[c * dim..(c + 1) * dim];
        let l1c = cc.iter().sum::<f32>();
        d.par_iter_mut().zip(fit.par_iter()).for_each(|(dd, &i)| {
            let (ri, rv) = set.row(i);
            *dd = dd.min(sdist(ri, rv, cc, l1c));
        });
    }
    let mut prev = vec![u16::MAX; nfit];
    for _ in 0..iters {
        let l1 = l1_norms(&cents, dim);
        let assign: Vec<u16> = fit.par_iter().map(|&i| snearest(set, i, &cents, &l1, dim).0 as u16).collect();
        let changed = assign.iter().zip(&prev).filter(|(a, b)| a != b).count();
        let mut sums = vec![0f64; k * dim];
        let mut cnt = vec![0f64; k];
        for (&i, &a) in fit.iter().zip(&assign) {
            let (ri, rv) = set.row(i);
            cnt[a as usize] += 1.0;
            for (&j, &v) in ri.iter().zip(rv) {
                sums[a as usize * dim + j as usize] += v as f64;
            }
        }
        for c in 0..k {
            if cnt[c] > 0.0 {
                for j in 0..dim {
                    cents[c * dim + j] = (sums[c * dim + j] / cnt[c]) as f32;
                }
            } else {
                densify(fit[rng.below(nfit as u32) as usize], &mut cents[c * dim..(c + 1) * dim]);
            }
        }
        prev = assign;
        if changed * 1000 < nfit {
            break;
        }
    }
    cents
}

// ---------------------------------------------------------------------------------------------
// Per-flop tables

/// Bucket tables of one canonical flop (suit-canonical card arguments).
pub struct FlopBuckets {
    pub flop_id: u16,
    pub flop: [Card; 3],
    pub k_turn: u16,
    pub k_river: u16,
    turn: Vec<u16>,
    river: Vec<u16>,
}

const MAGIC: &[u8; 4] = b"SBK1";
const HEADER: usize = 16;

impl FlopBuckets {
    /// Turn bucket of `hole` with turn card `turn` (cards in canonical suits).
    #[inline]
    pub fn turn(&self, hole: [Card; 2], turn: Card) -> u16 {
        self.turn[hole_index(hole[0], hole[1]) * 52 + turn as usize]
    }

    /// River bucket of `hole` with board flop + `turn` + `river` (order of turn/river irrelevant).
    #[inline]
    pub fn river(&self, hole: [Card; 2], turn: Card, river: Card) -> u16 {
        self.river[hole_index(hole[0], hole[1]) * NUM_HOLES + hole_index(turn, river)]
    }

    pub fn bytes(&self) -> usize {
        HEADER + 2 * (TURN_LEN + RIVER_LEN)
    }

    pub fn write(&self, path: &Path) -> io::Result<()> {
        let mut buf = Vec::with_capacity(self.bytes());
        buf.extend_from_slice(MAGIC);
        buf.extend_from_slice(&self.flop_id.to_le_bytes());
        buf.extend_from_slice(&self.flop);
        buf.push(0);
        buf.extend_from_slice(&self.k_turn.to_le_bytes());
        buf.extend_from_slice(&self.k_river.to_le_bytes());
        buf.extend_from_slice(&[0, 0]);
        debug_assert_eq!(buf.len(), HEADER);
        for &x in self.turn.iter().chain(&self.river) {
            buf.extend_from_slice(&x.to_le_bytes());
        }
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, &buf)?;
        fs::rename(tmp, path)
    }

    pub fn read(path: &Path) -> io::Result<Self> {
        let b = fs::read(path)?;
        let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
        if b.len() != HEADER + 2 * (TURN_LEN + RIVER_LEN) || &b[..4] != MAGIC {
            return Err(bad("bad flop bucket file"));
        }
        let u16at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
        let body: Vec<u16> = b[HEADER..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        Ok(FlopBuckets {
            flop_id: u16at(4),
            flop: [b[6], b[7], b[8]],
            k_turn: u16at(10),
            k_river: u16at(12),
            turn: body[..TURN_LEN].to_vec(),
            river: body[TURN_LEN..].to_vec(),
        })
    }
}

/// Build the tables of canonical flop `flop` (id only recorded in the result).
pub fn build_flop(flop_id: u16, flop: [Card; 3], opp: &[u8], p: &BuildParams) -> (FlopBuckets, BuildStats) {
    let mut stats = BuildStats::default();
    let hs = holes();
    let flop_mask = mask_of(&flop);
    let live: Vec<Card> = (0..52u8).filter(|&c| bit(c) & flop_mask == 0).collect();
    let seed = p.seed ^ (flop_id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);

    // ---- river: OCHS features of every (hole, turn/river pair)
    let t0 = Instant::now();
    let mut pairs: Vec<(Card, Card)> = Vec::with_capacity(live.len() * (live.len() - 1) / 2);
    for (a, &t) in live.iter().enumerate() {
        for &r in &live[a + 1..] {
            pairs.push((t, r));
        }
    }
    let per_pair: Vec<(Vec<u16>, Vec<f32>)> = pairs
        .par_iter()
        .map(|&(t, r)| {
            let board = flop_mask | bit(t) | bit(r);
            let mut out = vec![[0.5f32; OCHS_K]; NUM_HOLES];
            river_cluster_equities::<OCHS_K>(board, opp, &mut out);
            let mut hl = Vec::with_capacity(1081);
            let mut ft = Vec::with_capacity(1081 * OCHS_K);
            for (i, &h) in hs.iter().enumerate() {
                if hole_mask(h) & board == 0 {
                    hl.push(i as u16);
                    ft.extend_from_slice(&out[i]);
                }
            }
            (hl, ft)
        })
        .collect();
    let npts: usize = per_pair.iter().map(|(h, _)| h.len()).sum();
    let mut feats: Vec<f32> = Vec::with_capacity(npts * OCHS_K);
    let mut meta: Vec<(u16, u16)> = Vec::with_capacity(npts); // (hole index, pair index)
    for ((t, r), (hl, ft)) in pairs.iter().zip(per_pair) {
        let pi = hole_index(*t, *r) as u16;
        meta.extend(hl.iter().map(|&h| (h, pi)));
        feats.extend(ft);
    }
    stats.river_features_s = t0.elapsed().as_secs_f64();

    let t0 = Instant::now();
    let mut rng = Rng::new(seed);
    let nfit = p.river_fit_sample.min(npts);
    let mut sample = Vec::with_capacity(nfit * OCHS_K);
    for _ in 0..nfit {
        let i = rng.below(npts as u32) as usize;
        sample.extend_from_slice(&feats[i * OCHS_K..(i + 1) * OCHS_K]);
    }
    let (mut km, _) = KMeans::fit(&sample, None, OCHS_K, p.k_river, Metric::L2, p.river_iters, seed ^ 3);
    km.sort_by_strength();
    drop(sample);
    let assign: Vec<u16> = feats.par_chunks(OCHS_K).map(|f| km.nearest(f).0 as u16).collect();
    drop(feats);
    let mut river = vec![NO_BUCKET; RIVER_LEN];
    for (&(h, pi), &b) in meta.iter().zip(&assign) {
        river[h as usize * NUM_HOLES + pi as usize] = b;
    }
    drop((meta, assign));
    stats.river_cluster_s = t0.elapsed().as_secs_f64();

    // ---- turn: potential-aware histograms over river buckets
    let t0 = Instant::now();
    let tree = BucketTree::build(&km.centroids, OCHS_K, p.fanout, p.trees, seed ^ 5);
    stats.tree_corr = tree.corr;
    let per_turn: Vec<(Vec<u16>, SparseSet)> = live
        .par_iter()
        .map(|&t| {
            let dead = flop_mask | bit(t);
            let mut hl = Vec::new();
            let mut set = SparseSet::new();
            let mut row = Vec::new();
            for (i, &h) in hs.iter().enumerate() {
                if hole_mask(h) & dead != 0 {
                    continue;
                }
                let d2 = dead | hole_mask(h);
                let rivers = live.iter().filter(|&&r| bit(r) & d2 == 0).map(|&r| river[i * NUM_HOLES + hole_index(t, r)]);
                tree.embed(rivers, &mut row);
                set.push(&row);
                hl.push(i as u16);
            }
            (hl, set)
        })
        .collect();
    let mut set = SparseSet::new();
    let mut tmeta: Vec<(u16, u8)> = Vec::new();
    for (&t, (hl, s)) in live.iter().zip(per_turn) {
        for (i, &h) in hl.iter().enumerate() {
            let (ri, rv) = s.row(i);
            set.idx.extend_from_slice(ri);
            set.val.extend_from_slice(rv);
            set.offs.push(set.idx.len() as u32);
            tmeta.push((h, t));
        }
    }
    stats.turn_features_s = t0.elapsed().as_secs_f64();

    let t0 = Instant::now();
    let dim = tree.n_nodes();
    let n = set.len();
    let nfit = p.turn_fit_sample.min(n).max(p.k_turn);
    let fit: Vec<usize> = if nfit >= n { (0..n).collect() } else { (0..nfit).map(|_| rng.below(n as u32) as usize).collect() };
    let mut cents = sparse_kmeans(&set, &fit, dim, p.k_turn, p.turn_iters, seed ^ 11);
    // Order turn buckets by the mean river-bucket id of their histogram (strength proxy).
    let leaf0 = tree.leaf0;
    let key: Vec<f32> = cents
        .chunks_exact(dim)
        .map(|c| (0..tree.k).map(|b| c[leaf0 + b] / tree.weight[leaf0 + b] * b as f32).sum())
        .collect();
    let mut order: Vec<usize> = (0..p.k_turn).collect();
    order.sort_by(|&a, &b| key[a].partial_cmp(&key[b]).unwrap());
    let mut sorted = Vec::with_capacity(cents.len());
    for &o in &order {
        sorted.extend_from_slice(&cents[o * dim..(o + 1) * dim]);
    }
    cents = sorted;
    let l1 = l1_norms(&cents, dim);
    let tassign: Vec<u16> = (0..n).into_par_iter().map(|i| snearest(&set, i, &cents, &l1, dim).0 as u16).collect();
    let mut turn = vec![NO_BUCKET; TURN_LEN];
    for (&(h, t), &b) in tmeta.iter().zip(&tassign) {
        turn[h as usize * 52 + t as usize] = b;
    }
    stats.turn_cluster_s = t0.elapsed().as_secs_f64();

    (FlopBuckets { flop_id, flop, k_turn: p.k_turn as u16, k_river: p.k_river as u16, turn, river }, stats)
}

// ---------------------------------------------------------------------------------------------
// Lookup front-end

/// Bucket lookup for one concrete flop (any suits).
#[derive(Clone)]
pub struct FlopView {
    pub fb: Arc<FlopBuckets>,
    perm: [u8; 4],
}

impl FlopView {
    /// Bucket of `hole` on `board` = flop + turn (+ river). `None` for boards of other lengths
    /// (search is lossless on the current round and never needs flop/preflop buckets here).
    #[inline]
    pub fn bucket(&self, hole: [Card; 2], board: &[Card]) -> Option<u32> {
        let m = |c: Card| map_card(c, &self.perm);
        debug_assert_eq!(mapped_sorted([board[0], board[1], board[2]], &self.perm), self.fb.flop, "board does not start with this flop");
        let h = [m(hole[0]), m(hole[1])];
        match board.len() {
            4 => Some(self.fb.turn(h, m(board[3])) as u32),
            5 => Some(self.fb.river(h, m(board[3]), m(board[4])) as u32),
            _ => None,
        }
    }

    pub fn n_buckets(&self, board_len: usize) -> u32 {
        match board_len {
            4 => self.fb.k_turn as u32,
            5 => self.fb.k_river as u32,
            _ => 0,
        }
    }
}

type Cell = Arc<OnceLock<Arc<FlopBuckets>>>;

/// All flops' tables behind an LRU cache; each flop is loaded from `dir` if present, else
/// built on first use (and written to `dir` when one is configured).
pub struct SearchBuckets {
    canon: FlopCanon,
    dir: Option<PathBuf>,
    pub params: BuildParams,
    opp: Vec<u8>,
    cap: usize,
    cache: Mutex<HashMap<u16, (u64, Cell)>>,
    tick: AtomicU64,
    pub builds: AtomicU64,
    pub loads: AtomicU64,
    pub build_micros: AtomicU64,
}

impl fmt::Debug for SearchBuckets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SearchBuckets({}/{} buckets, dir {:?})", self.params.k_turn, self.params.k_river, self.dir)
    }
}

impl SearchBuckets {
    /// `dir`: precomputed tables (and where lazily built ones are saved); `None` = memory only.
    pub fn new(dir: Option<PathBuf>, params: BuildParams, cache_cap: usize) -> io::Result<Self> {
        QUIET.store(true, Relaxed);
        let opp = match &dir {
            Some(d) => {
                fs::create_dir_all(d)?;
                match fs::read(d.join("opp.u8")) {
                    Ok(b) if b.len() == NUM_HOLES => b,
                    _ => {
                        let o = opponent_clusters(params.opp_samples, params.seed);
                        fs::write(d.join("opp.u8"), &o)?;
                        o
                    }
                }
            }
            None => opponent_clusters(params.opp_samples, params.seed),
        };
        Ok(SearchBuckets {
            canon: FlopCanon::new(),
            dir,
            params,
            opp,
            cap: cache_cap.max(1),
            cache: Mutex::new(HashMap::new()),
            tick: AtomicU64::new(0),
            builds: AtomicU64::new(0),
            loads: AtomicU64::new(0),
            build_micros: AtomicU64::new(0),
        })
    }

    pub fn canon(&self) -> &FlopCanon {
        &self.canon
    }

    pub fn flop_path(&self, id: u16) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(format!("flop-{id:04}.sb")))
    }

    /// Build (never loads) canonical flop `id`.
    pub fn build_id(&self, id: u16) -> (FlopBuckets, BuildStats) {
        build_flop(id, self.canon.flop(id), &self.opp, &self.params)
    }

    fn load_or_build(&self, id: u16) -> FlopBuckets {
        if let Some(path) = self.flop_path(id) {
            if let Ok(fb) = FlopBuckets::read(&path) {
                if fb.flop_id == id && fb.k_turn as usize == self.params.k_turn && fb.k_river as usize == self.params.k_river {
                    self.loads.fetch_add(1, Relaxed);
                    return fb;
                }
            }
        }
        let t0 = Instant::now();
        let (fb, _) = self.build_id(id);
        self.builds.fetch_add(1, Relaxed);
        self.build_micros.fetch_add(t0.elapsed().as_micros() as u64, Relaxed);
        if let Some(path) = self.flop_path(id) {
            let _ = fb.write(&path);
        }
        fb
    }

    /// Tables for canonical flop `id` (cached).
    pub fn flop_buckets(&self, id: u16) -> Arc<FlopBuckets> {
        let cell = {
            let mut g = self.cache.lock().unwrap();
            let t = self.tick.fetch_add(1, Relaxed) + 1;
            let e = g.entry(id).or_insert_with(|| (t, Arc::new(OnceLock::new())));
            e.0 = t;
            let cell = e.1.clone();
            if g.len() > self.cap {
                if let Some((&victim, _)) = g.iter().filter(|(k, _)| **k != id).min_by_key(|(_, v)| v.0) {
                    g.remove(&victim);
                }
            }
            cell
        };
        cell.get_or_init(|| Arc::new(self.load_or_build(id))).clone()
    }

    /// Bucket lookup front-end for the flop `flop` (any suits, any order).
    pub fn for_flop(&self, flop: [Card; 3]) -> FlopView {
        let (id, perm) = self.canon.canonicalize(flop);
        FlopView { fb: self.flop_buckets(id), perm }
    }
}

// ---------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::equity::river_equities;
    use cards::parse_cards;

    #[test]
    fn there_are_1755_flops_and_canonicalisation_is_suit_invariant() {
        let c = FlopCanon::new();
        assert_eq!(c.len(), NUM_FLOPS);
        let mut rng = Rng::new(3);
        for _ in 0..200 {
            let mut d = 0;
            let mut f = [0u8; 3];
            for x in f.iter_mut() {
                *x = rng.draw(d);
                d |= bit(*x);
            }
            let (id, perm) = c.canonicalize(f);
            assert_eq!(mapped_sorted(f, &perm), c.flop(id));
            for p in suit_perms().iter().step_by(5) {
                let g = [map_card(f[0], p), map_card(f[1], p), map_card(f[2], p)];
                assert_eq!(c.canonicalize(g).0, id);
            }
        }
    }

    #[test]
    fn tree_distance_respects_hierarchy() {
        let mut rng = Rng::new(9);
        // 3 well-separated blobs of 20 points in 4-D.
        let mut pts = Vec::new();
        for i in 0..60 {
            let c = (i / 20) as f32 * 10.0;
            for _ in 0..4 {
                pts.push(c + rng.f32());
            }
        }
        let t = BucketTree::build(&pts, 4, 5, 2, 1);
        assert!(t.corr > 0.7, "corr {}", t.corr);
        assert_eq!(t.distance(3, 3), 0.0);
        assert!((t.distance(3, 40) - t.distance(40, 3)).abs() < 1e-6);
        assert!(t.distance(1, 2) < t.distance(1, 45));
        // Embedded L1 equals tree EMD between point masses.
        let (mut a, mut b) = (vec![], vec![]);
        t.embed([4u16].into_iter(), &mut a);
        t.embed([50u16].into_iter(), &mut b);
        let mut dense = vec![0f32; t.n_nodes()];
        a.iter().for_each(|&(j, v)| dense[j as usize] += v);
        b.iter().for_each(|&(j, v)| dense[j as usize] -= v);
        let l1: f32 = dense.iter().map(|x| x.abs()).sum();
        assert!((l1 - t.distance(4, 50)).abs() < 1e-4);
    }

    #[test]
    fn sparse_kmeans_separates() {
        let mut set = SparseSet::new();
        for i in 0..600u16 {
            let base = (i % 3) * 4;
            set.push(&[(base, 0.7), (base + 1, 0.3)]);
        }
        let fit: Vec<usize> = (0..600).collect();
        let c = sparse_kmeans(&set, &fit, 12, 3, 10, 5);
        let l1 = l1_norms(&c, 12);
        for i in 0..600 {
            assert_eq!(snearest(&set, i, &c, &l1, 12).0, snearest(&set, i % 3, &c, &l1, 12).0);
        }
    }

    #[test]
    fn tiny_build_is_complete_and_meaningful() {
        let sb = SearchBuckets::new(None, BuildParams::tiny(), 4).unwrap();
        let flop = parse_cards("Ts7d2c").unwrap();
        let view = sb.for_flop([flop[0], flop[1], flop[2]]);
        let fb = &view.fb;
        // every valid combination has a bucket < k, invalid ones none
        let hs = holes();
        let fm = mask_of(&fb.flop);
        let (mut turn_ok, mut river_ok) = (0usize, 0usize);
        for (i, &h) in hs.iter().enumerate() {
            for t in 0..52u8 {
                let v = fb.turn[i * 52 + t as usize];
                let valid = bit(t) & fm == 0 && hole_mask(h) & (fm | bit(t)) == 0;
                assert_eq!(v != NO_BUCKET, valid);
                if valid {
                    assert!(v < fb.k_turn);
                    turn_ok += 1;
                }
            }
        }
        assert_eq!(turn_ok, 49 * 1128);
        for &v in &fb.river {
            if v != NO_BUCKET {
                assert!(v < fb.k_river);
                river_ok += 1;
            }
        }
        assert_eq!(river_ok, 1176 * 1081);

        // River buckets explain exact equity; bucket ids rise with strength.
        let live: Vec<Card> = (0..52u8).filter(|&c| bit(c) & fm == 0).collect();
        let (t, r) = (live[10], live[30]);
        let board5 = fm | bit(t) | bit(r);
        let mut eq = [0f32; NUM_HOLES];
        river_equities(board5, &mut eq);
        let k = fb.k_river as usize;
        let (mut s, mut s2, mut n) = (vec![0f64; k], vec![0f64; k], vec![0f64; k]);
        for (i, &h) in hs.iter().enumerate() {
            if hole_mask(h) & board5 == 0 {
                let b = fb.river(h, t, r) as usize;
                s[b] += eq[i] as f64;
                s2[b] += (eq[i] * eq[i]) as f64;
                n[b] += 1.0;
            }
        }
        let tot_n: f64 = n.iter().sum();
        let mean = s.iter().sum::<f64>() / tot_n;
        let total = s2.iter().sum::<f64>() / tot_n - mean * mean;
        let within: f64 = (0..k).filter(|&b| n[b] > 0.0).map(|b| s2[b] - s[b] * s[b] / n[b]).sum::<f64>() / tot_n;
        assert!(1.0 - within / total > 0.8, "river R2 {}", 1.0 - within / total);
        let means: Vec<f64> = (0..k).filter(|&b| n[b] > 0.0).map(|b| s[b] / n[b]).collect();
        assert!(means[0] < means[means.len() - 1] - 0.3);

        // Suit-permuted copy of the same situation maps to the same buckets.
        let p = [2u8, 0, 3, 1];
        let mp = |c: Card| map_card(c, &p);
        let view2 = sb.for_flop([mp(flop[0]), mp(flop[1]), mp(flop[2])]);
        assert_eq!(view2.fb.flop_id, view.fb.flop_id);
        let hole = [parse_cards("As").unwrap()[0], parse_cards("Qs").unwrap()[0]];
        let b1 = [flop[0], flop[1], flop[2], parse_cards("9h").unwrap()[0], parse_cards("Kc").unwrap()[0]];
        let b2: Vec<Card> = b1.iter().map(|&c| mp(c)).collect();
        assert_eq!(view.bucket(hole, &b1), view2.bucket([mp(hole[0]), mp(hole[1])], &b2));
        assert_eq!(view.bucket(hole, &b1[..4]), view2.bucket([mp(hole[0]), mp(hole[1])], &b2[..4]));
        assert_eq!(view.bucket(hole, &b1[..3]), None);
    }

    #[test]
    fn file_roundtrip_and_lru() {
        let dir = std::env::temp_dir().join(format!("sb-test-{}", std::process::id()));
        let sb = SearchBuckets::new(Some(dir.clone()), BuildParams::tiny(), 1).unwrap();
        let a = sb.flop_buckets(0);
        let _b = sb.flop_buckets(1); // evicts 0
        assert_eq!(sb.builds.load(Relaxed), 2);
        let a2 = sb.flop_buckets(0); // reloaded from disk
        assert_eq!(sb.loads.load(Relaxed), 1);
        assert_eq!(a.turn, a2.turn);
        assert_eq!(a.river, a2.river);
        let _ = fs::remove_dir_all(dir);
    }
}
