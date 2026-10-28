//! Parallel k-means with k-means++ seeding, for two metrics:
//! - `Metric::L2` on raw feature vectors (river OCHS),
//! - `Metric::Emd1d` on histograms over ordered bins. For 1-D histograms with unit ground
//!   distance, earth mover's distance equals the L1 distance between CDFs, so points are
//!   converted to CDFs once and centroids are means of CDFs (still valid CDFs).
//!
//! Points are stored row-major in one flat `Vec<f32>`.

use cards::Rng;
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// Silence the per-iteration progress lines (set by bulk builders that run thousands of fits).
pub static QUIET: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metric {
    L2,
    Emd1d,
}

pub struct KMeans {
    pub k: usize,
    pub dim: usize,
    pub centroids: Vec<f32>,
    pub metric: Metric,
}

/// Normalise a histogram and convert it to a CDF in place.
pub fn to_cdf(h: &mut [f32]) {
    let total: f32 = h.iter().sum();
    let mut acc = 0.0;
    for x in h.iter_mut() {
        acc += if total > 0.0 { *x / total } else { 0.0 };
        *x = acc;
    }
}

#[inline]
fn dist(metric: Metric, a: &[f32], b: &[f32]) -> f32 {
    match metric {
        Metric::L2 => a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum(),
        Metric::Emd1d => a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum(),
    }
}

impl KMeans {
    /// Index of the nearest centroid and its distance.
    #[inline]
    pub fn nearest(&self, p: &[f32]) -> (usize, f32) {
        let mut best = (0, f32::INFINITY);
        for (c, cent) in self.centroids.chunks_exact(self.dim).enumerate() {
            let d = dist(self.metric, p, cent);
            if d < best.1 {
                best = (c, d);
            }
        }
        best
    }

    /// Fit on `points` (row-major, `dim` columns, already CDF-transformed for `Emd1d`), with
    /// optional per-point `weights` (e.g. how many raw situations a canonical point stands for).
    /// Seeding uses k-means++ on a random sample of at most `seed_sample` points.
    pub fn fit(points: &[f32], weights: Option<&[f32]>, dim: usize, k: usize, metric: Metric, iters: usize, seed: u64) -> (Self, Vec<u16>) {
        let n = points.len() / dim;
        assert!(n >= k && k <= u16::MAX as usize);
        let w = |i: usize| weights.map_or(1.0, |w| w[i]);
        let mut rng = Rng::new(seed);
        let seed_sample = 20_000.min(n);
        let sample: Vec<usize> = (0..seed_sample).map(|_| rng.below(n as u32) as usize).collect();
        let row = |i: usize| &points[i * dim..(i + 1) * dim];

        // k-means++ on the sample.
        let mut centroids = Vec::with_capacity(k * dim);
        centroids.extend_from_slice(row(sample[0]));
        let mut d2: Vec<f32> = sample.iter().map(|&i| dist(metric, row(i), &centroids[..dim])).collect();
        for _ in 1..k {
            let probs: Vec<f32> = sample.iter().zip(&d2).map(|(&i, &d)| w(i) * if metric == Metric::L2 { d } else { d * d }).collect();
            let pick = sample[rng.sample_weighted(&probs)];
            let start = centroids.len();
            centroids.extend_from_slice(row(pick));
            let newc = centroids[start..].to_vec();
            for (j, &i) in sample.iter().enumerate() {
                d2[j] = d2[j].min(dist(metric, row(i), &newc));
            }
        }
        let mut km = KMeans { k, dim, centroids, metric };
        let mut assign = vec![0u16; n];

        for it in 0..iters {
            let chunk = 1 << 14;
            let partial: Vec<(Vec<f64>, Vec<f64>, f64, usize)> = assign
                .par_chunks_mut(chunk)
                .enumerate()
                .map(|(ci, a)| {
                    let mut sums = vec![0f64; k * dim];
                    let mut cnt = vec![0f64; k];
                    let (mut cost, mut changed) = (0f64, 0usize);
                    for (o, slot) in a.iter_mut().enumerate() {
                        let i = ci * chunk + o;
                        let p = row(i);
                        let (c, d) = km.nearest(p);
                        if *slot as usize != c {
                            changed += 1;
                        }
                        *slot = c as u16;
                        let wi = w(i) as f64;
                        cost += d as f64 * wi;
                        cnt[c] += wi;
                        for (s, &x) in sums[c * dim..(c + 1) * dim].iter_mut().zip(p) {
                            *s += x as f64 * wi;
                        }
                    }
                    (sums, cnt, cost, changed)
                })
                .collect();
            let mut sums = vec![0f64; k * dim];
            let mut cnt = vec![0f64; k];
            let (mut cost, mut changed) = (0f64, 0usize);
            for (s, c, co, ch) in partial {
                sums.iter_mut().zip(s).for_each(|(a, b)| *a += b);
                cnt.iter_mut().zip(c).for_each(|(a, b)| *a += b);
                cost += co;
                changed += ch;
            }
            for c in 0..k {
                if cnt[c] > 0.0 {
                    for d in 0..dim {
                        km.centroids[c * dim + d] = (sums[c * dim + d] / cnt[c]) as f32;
                    }
                } else {
                    // Re-seed an empty cluster at a random point.
                    let i = rng.below(n as u32) as usize;
                    km.centroids[c * dim..(c + 1) * dim].copy_from_slice(row(i));
                }
            }
            if !QUIET.load(Ordering::Relaxed) {
                eprintln!("  kmeans iter {it}: cost {:.6} changed {changed}", cost / n as f64);
            }
            if it > 0 && changed == 0 {
                break;
            }
        }
        (km, assign)
    }

    /// Reorder clusters so that bucket ids increase with the centroid's mean value
    /// (for CDFs: decreasing CDF mass = increasing strength). Returns old->new map.
    pub fn sort_by_strength(&mut self) -> Vec<u16> {
        let key = |c: usize| -> f32 {
            let cent = &self.centroids[c * self.dim..(c + 1) * self.dim];
            match self.metric {
                Metric::L2 => cent.iter().sum::<f32>(),
                Metric::Emd1d => -cent.iter().sum::<f32>(),
            }
        };
        let mut order: Vec<usize> = (0..self.k).collect();
        order.sort_by(|&a, &b| key(a).partial_cmp(&key(b)).unwrap());
        let mut map = vec![0u16; self.k];
        let mut cents = Vec::with_capacity(self.centroids.len());
        for (new, &old) in order.iter().enumerate() {
            map[old] = new as u16;
            cents.extend_from_slice(&self.centroids[old * self.dim..(old + 1) * self.dim]);
        }
        self.centroids = cents;
        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separates_obvious_clusters() {
        let mut rng = Rng::new(1);
        let mut pts = Vec::new();
        for i in 0..3000 {
            let center = (i % 3) as f32 * 10.0;
            pts.push(center + rng.f32());
            pts.push(center + rng.f32());
        }
        let (km, assign) = KMeans::fit(&pts, None, 2, 3, Metric::L2, 20, 7);
        assert_eq!(km.k, 3);
        for i in 0..3000 {
            assert_eq!(assign[i], assign[i % 3]);
        }
    }

    #[test]
    fn emd_is_cdf_l1() {
        let mut a = vec![1.0, 0.0, 0.0, 0.0];
        let mut b = vec![0.0, 0.0, 0.0, 1.0];
        to_cdf(&mut a);
        to_cdf(&mut b);
        assert_eq!(dist(Metric::Emd1d, &a, &b), 3.0);
    }
}
