//! All-in equity EV: replace the realised runout by its expectation.
//!
//! Given every live player's hole cards and the board known when betting stopped, the
//! expected net chips of each seat follow from enumerating (<= 2 missing cards) or sampling
//! (preflop, 5 missing) the runouts and splitting each side-pot layer among the best hands.
//! This removes *runout luck* only; see the README for what it cannot correct.

use cards::{bit, evaluate, mask_of, Card, Hand, Rng};

#[derive(Clone, Debug)]
pub struct Entry {
    pub seat: usize,
    /// Total chips committed to the pot over the hand.
    pub contrib: i64,
    pub live: bool,
    pub hole: Option<[Card; 2]>,
}

struct Layer {
    amount: f64,
    /// Indices into the live list.
    eligible: Vec<usize>,
}

/// Expected net chips per seat (payout expectation minus contribution), or `None` when a live
/// player's cards are unknown or the inputs are inconsistent.
pub fn expected_net(entries: &[Entry], board: &[Card], mc_samples: usize, seed: u64) -> Option<Vec<(usize, f64)>> {
    if board.len() > 5 || (!board.is_empty() && board.len() < 3) {
        return None;
    }
    let live: Vec<&Entry> = entries.iter().filter(|e| e.live).collect();
    if live.len() < 2 {
        return None;
    }
    let holes: Vec<[Card; 2]> = live.iter().map(|e| e.hole).collect::<Option<Vec<_>>>()?;
    let mut dead: Hand = mask_of(board);
    if dead.count_ones() as usize != board.len() {
        return None;
    }
    for h in &holes {
        for &c in h {
            if dead & bit(c) != 0 {
                return None; // duplicate card: inconsistent input
            }
            dead |= bit(c);
        }
    }
    // Side-pot layers over all contributions (folded money included).
    let mut levels: Vec<i64> = live.iter().map(|e| e.contrib).collect();
    levels.sort_unstable();
    levels.dedup();
    let mut layers = Vec::new();
    let mut prev = 0i64;
    for lv in levels {
        if lv <= prev {
            continue;
        }
        let amount: i64 = entries.iter().map(|e| e.contrib.min(lv) - e.contrib.min(prev)).sum();
        let eligible: Vec<usize> = live
            .iter()
            .enumerate()
            .filter(|(_, e)| e.contrib >= lv)
            .map(|(i, _)| i)
            .collect();
        layers.push(Layer { amount: amount as f64, eligible });
        prev = lv;
    }
    let m = 5 - board.len();
    let remaining: Vec<Card> = (0..52u8).filter(|&c| dead & bit(c) == 0).collect();
    let mut acc = vec![0f64; live.len()];
    let mut count = 0f64;
    let eval_runout = |extra: &[Card], acc: &mut Vec<f64>| {
        let bm = mask_of(board) | mask_of(extra);
        let ranks: Vec<u32> = holes.iter().map(|h| evaluate(bm | bit(h[0]) | bit(h[1]))).collect();
        for l in &layers {
            let best = l.eligible.iter().map(|&i| ranks[i]).max().unwrap();
            let w: Vec<usize> = l.eligible.iter().copied().filter(|&i| ranks[i] == best).collect();
            let share = l.amount / w.len() as f64;
            for i in w {
                acc[i] += share;
            }
        }
    };
    match m {
        0 => {
            eval_runout(&[], &mut acc);
            count = 1.0;
        }
        1 => {
            for &a in &remaining {
                eval_runout(&[a], &mut acc);
                count += 1.0;
            }
        }
        2 => {
            for (i, &a) in remaining.iter().enumerate() {
                for &b in &remaining[i + 1..] {
                    eval_runout(&[a, b], &mut acc);
                    count += 1.0;
                }
            }
        }
        _ => {
            let mut rng = Rng::new(seed);
            for _ in 0..mc_samples.max(1) {
                let mut d = dead;
                let mut extra = Vec::with_capacity(m);
                for _ in 0..m {
                    let c = rng.draw(d);
                    d |= bit(c);
                    extra.push(c);
                }
                eval_runout(&extra, &mut acc);
                count += 1.0;
            }
        }
    }
    let mut out = Vec::new();
    for e in entries {
        let payout = live
            .iter()
            .position(|l| l.seat == e.seat)
            .map_or(0.0, |i| acc[i] / count);
        out.push((e.seat, payout - e.contrib as f64));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cards::parse_cards;

    fn h(s: &str) -> [Card; 2] {
        let v = parse_cards(s).unwrap();
        [v[0], v[1]]
    }
    fn e(seat: usize, contrib: i64, live: bool, hole: Option<&str>) -> Entry {
        Entry { seat, contrib, live, hole: hole.map(h) }
    }
    fn net(v: &[(usize, f64)], seat: usize) -> f64 {
        v.iter().find(|x| x.0 == seat).unwrap().1
    }

    #[test]
    fn turn_lock_is_exact() {
        // AA vs KK on Qc 7d 2h 3s: KK wins only on the 2 remaining kings out of 44 cards.
        let b = parse_cards("Qc7d2h3s").unwrap();
        let r = expected_net(&[e(0, 100, true, Some("AhAd")), e(1, 100, true, Some("KsKc"))], &b, 0, 1).unwrap();
        let aa = 200.0 * 42.0 / 44.0 - 100.0;
        assert!((net(&r, 0) - aa).abs() < 1e-9, "{r:?}");
        assert!((net(&r, 0) + net(&r, 1)).abs() < 1e-9, "zero-sum");
    }

    #[test]
    fn river_is_deterministic_and_ties_split() {
        let b = parse_cards("AsKsQsJs2d").unwrap();
        let r = expected_net(&[e(0, 50, true, Some("9c9d")), e(1, 50, true, Some("8c8d"))], &b, 0, 1).unwrap();
        assert!((net(&r, 0) - 50.0).abs() < 1e-9 && (net(&r, 1) + 50.0).abs() < 1e-9);
        let r = expected_net(&[e(0, 50, true, Some("3c4d")), e(1, 50, true, Some("3d4c"))], &b, 0, 1).unwrap();
        assert!(net(&r, 0).abs() < 1e-9 && net(&r, 1).abs() < 1e-9);
    }

    #[test]
    fn side_pots_and_dead_money() {
        // Seat 2 all-in for 40 with the best hand, seats 0/1 put in 100 each, seat 3 folded 20.
        let b = parse_cards("2c7d9hJs3c").unwrap();
        let r = expected_net(
            &[
                e(0, 100, true, Some("KcKd")),
                e(1, 100, true, Some("QcQd")),
                e(2, 40, true, Some("AcAd")),
                e(3, 20, false, None),
            ],
            &b,
            0,
            1,
        )
        .unwrap();
        // Main pot 40*3 + 20 = 140 -> seat 2. Side pot 60*2 = 120 -> seat 0 (KK > QQ).
        assert!((net(&r, 2) - (140.0 - 40.0)).abs() < 1e-9);
        assert!((net(&r, 0) - (120.0 - 100.0)).abs() < 1e-9);
        assert!((net(&r, 1) + 100.0).abs() < 1e-9);
        assert!((net(&r, 3) + 20.0).abs() < 1e-9);
        let total: f64 = r.iter().map(|x| x.1).sum();
        assert!(total.abs() < 1e-9);
    }

    #[test]
    fn preflop_monte_carlo_matches_known_equity() {
        // AhAd vs KsKc preflop: ~81.9% for aces.
        let r = expected_net(&[e(0, 100, true, Some("AhAd")), e(1, 100, true, Some("KsKc"))], &[], 60_000, 7).unwrap();
        let eq = (net(&r, 0) + 100.0) / 200.0;
        assert!((eq - 0.819).abs() < 0.01, "equity {eq}");
    }

    #[test]
    fn unknown_cards_or_dupes_give_none() {
        let b = parse_cards("2c7d9h").unwrap();
        assert!(expected_net(&[e(0, 10, true, Some("AhAd")), e(1, 10, true, None)], &b, 0, 1).is_none());
        assert!(expected_net(&[e(0, 10, true, Some("AhAd")), e(1, 10, true, Some("AhKd"))], &b, 0, 1).is_none());
        assert!(expected_net(&[e(0, 10, true, Some("AhAd")), e(1, 10, false, None)], &b, 0, 1).is_none());
    }
}
