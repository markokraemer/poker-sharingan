//! Exact showdown equities with card removal.
//!
//! The core primitive ranks every live hole pair on a 5-card board once, sorts, and then
//! derives each hand's win/tie counts against every opponent pair in O(1) using per-card
//! inclusion–exclusion counters. Opponent pairs can be split into `K` clusters, which gives
//! OCHS features ("opponent cluster hand strength", Johanson et al. 2013) for free;
//! `K = 1` is plain equity against a uniformly random hand.

use cards::{all_holes, bit, evaluate, Card, Hand};
use std::sync::OnceLock;

pub const NUM_HOLES: usize = 1326;

pub fn holes() -> &'static [[Card; 2]] {
    static H: OnceLock<Vec<[Card; 2]>> = OnceLock::new();
    H.get_or_init(all_holes)
}

#[inline(always)]
pub fn hole_mask(h: [Card; 2]) -> Hand {
    bit(h[0]) | bit(h[1])
}

/// Equity of every hole pair on a 5-card `board` against an opponent pair drawn uniformly from
/// the remaining cards, split by opponent cluster. `cluster[i]` is the cluster (`< K`) of hole
/// `i`. `out[i][c]` = (wins + ties/2) / matchups vs cluster `c`; holes that collide with the
/// board, and empty clusters, are left untouched. Returns nothing; reuses caller buffers.
pub fn river_cluster_equities<const K: usize>(board: Hand, cluster: &[u8], out: &mut [[f32; K]]) {
    debug_assert_eq!(board.count_ones(), 5);
    let hs = holes();
    let mut ranked: Vec<(u32, u16)> = Vec::with_capacity(1081);
    for (i, &h) in hs.iter().enumerate() {
        let m = hole_mask(h);
        if m & board == 0 {
            ranked.push((evaluate(board | m), i as u16));
        }
    }
    ranked.sort_unstable();

    // Counters: [c] totals and [card][c] per-card totals for "all live", "strictly less" and
    // "current equal-rank group".
    let mut all = [0u32; K];
    let mut all_card = vec![[0u32; K]; 52];
    for &(_, i) in &ranked {
        let c = cluster[i as usize] as usize;
        let h = hs[i as usize];
        all[c] += 1;
        all_card[h[0] as usize][c] += 1;
        all_card[h[1] as usize][c] += 1;
    }
    let mut less = [0u32; K];
    let mut less_card = vec![[0u32; K]; 52];
    let mut grp_card = vec![[0u32; K]; 52];

    let mut start = 0;
    while start < ranked.len() {
        let r = ranked[start].0;
        let mut end = start;
        let mut grp = [0u32; K];
        while end < ranked.len() && ranked[end].0 == r {
            let i = ranked[end].1 as usize;
            let c = cluster[i] as usize;
            let h = hs[i];
            grp[c] += 1;
            grp_card[h[0] as usize][c] += 1;
            grp_card[h[1] as usize][c] += 1;
            end += 1;
        }
        for &(_, i) in &ranked[start..end] {
            let (a, b) = (hs[i as usize][0] as usize, hs[i as usize][1] as usize);
            let own = cluster[i as usize] as usize;
            for c in 0..K {
                let me = (c == own) as u32;
                let n = all[c] - all_card[a][c] - all_card[b][c] + me;
                if n == 0 {
                    continue;
                }
                let w = less[c] - less_card[a][c] - less_card[b][c];
                let t = grp[c] - grp_card[a][c] - grp_card[b][c] + me;
                out[i as usize][c] = (w as f32 + 0.5 * t as f32) / n as f32;
            }
        }
        for &(_, i) in &ranked[start..end] {
            let c = cluster[i as usize] as usize;
            let h = hs[i as usize];
            less[c] += 1;
            less_card[h[0] as usize][c] += 1;
            less_card[h[1] as usize][c] += 1;
            grp_card[h[0] as usize][c] = 0;
            grp_card[h[1] as usize][c] = 0;
        }
        start = end;
    }
}

/// Equity of every hole pair on a 5-card board vs a uniformly random opponent pair.
/// Entries for holes that collide with the board are set to NaN.
pub fn river_equities(board: Hand, out: &mut [f32; NUM_HOLES]) {
    static ZERO: [u8; NUM_HOLES] = [0; NUM_HOLES];
    let mut tmp = vec![[f32::NAN; 1]; NUM_HOLES];
    river_cluster_equities::<1>(board, &ZERO, &mut tmp);
    for (o, t) in out.iter_mut().zip(tmp) {
        *o = t[0];
    }
}

/// For each hole pair, the histogram (`bins` equal-width bins over [0,1]) of its final river
/// equity over every possible runout of the remaining board cards. `board` has 3 or 4 cards.
/// Holes colliding with the board get an all-zero histogram. Counts are unnormalised.
pub fn runout_histograms(board: Hand, bins: usize) -> Vec<Vec<u16>> {
    let n_board = board.count_ones();
    assert!(n_board == 3 || n_board == 4);
    let mut hist = vec![vec![0u16; bins]; NUM_HOLES];
    let mut eq = [0f32; NUM_HOLES];
    let live: Vec<Card> = (0..52u8).filter(|&c| bit(c) & board == 0).collect();
    let mut add = |full: Hand, hist: &mut Vec<Vec<u16>>| {
        river_equities(full, &mut eq);
        for (i, &e) in eq.iter().enumerate() {
            if !e.is_nan() {
                let b = ((e * bins as f32) as usize).min(bins - 1);
                hist[i][b] += 1;
            }
        }
    };
    if n_board == 4 {
        for &r in &live {
            add(board | bit(r), &mut hist);
        }
    } else {
        for (x, &t) in live.iter().enumerate() {
            for &r in &live[x + 1..] {
                add(board | bit(t) | bit(r), &mut hist);
            }
        }
    }
    hist
}

/// Brute-force equity of one hand on a full board (test oracle).
pub fn equity_bruteforce(hole: [Card; 2], board: Hand) -> f32 {
    let me = evaluate(board | hole_mask(hole));
    let dead = board | hole_mask(hole);
    let (mut w, mut n) = (0f64, 0u32);
    for &o in holes() {
        let m = hole_mask(o);
        if m & dead != 0 {
            continue;
        }
        let r = evaluate(board | m);
        n += 1;
        w += if me > r { 1.0 } else if me == r { 0.5 } else { 0.0 };
    }
    (w / n as f64) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use cards::{mask_of, parse_cards, Rng};

    #[test]
    fn river_matches_bruteforce() {
        let mut rng = Rng::new(5);
        let mut eq = [0f32; NUM_HOLES];
        for _ in 0..20 {
            let mut board = 0;
            for _ in 0..5 {
                board |= bit(rng.draw(board));
            }
            river_equities(board, &mut eq);
            for (i, &h) in holes().iter().enumerate() {
                if hole_mask(h) & board != 0 {
                    assert!(eq[i].is_nan());
                } else {
                    let bf = equity_bruteforce(h, board);
                    assert!((eq[i] - bf).abs() < 1e-5, "{} vs {}", eq[i], bf);
                }
            }
        }
    }

    #[test]
    fn clusters_partition_matchups() {
        // With clusters, total wins/ties recombine to the K=1 equity.
        let board = mask_of(&parse_cards("2c7d9hJsKs").unwrap());
        let cluster: Vec<u8> = (0..NUM_HOLES).map(|i| (i % 3) as u8).collect();
        let mut by_c = vec![[f32::NAN; 3]; NUM_HOLES];
        river_cluster_equities::<3>(board, &cluster, &mut by_c);
        let mut eq = [0f32; NUM_HOLES];
        river_equities(board, &mut eq);
        for (i, &h) in holes().iter().enumerate() {
            if hole_mask(h) & board != 0 {
                continue;
            }
            let dead = board | hole_mask(h);
            let mut n = [0f32; 3];
            for (j, &o) in holes().iter().enumerate() {
                if hole_mask(o) & dead == 0 {
                    n[cluster[j] as usize] += 1.0;
                }
            }
            let total: f32 = (0..3).map(|c| by_c[i][c] * n[c]).sum::<f32>() / n.iter().sum::<f32>();
            assert!((total - eq[i]).abs() < 1e-4);
        }
    }

    #[test]
    fn turn_histograms_count_runouts() {
        let board = mask_of(&parse_cards("2c7d9hJs").unwrap());
        let h = runout_histograms(board, 10);
        let aa = cards::hole_index(cards::parse_card("Ac").unwrap(), cards::parse_card("Ad").unwrap());
        assert_eq!(h[aa].iter().map(|&x| x as u32).sum::<u32>(), 46);
    }
}
