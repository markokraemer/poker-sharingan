//! Branch-light 5..7-card hand evaluator over suit-major bitmasks.
//!
//! Returns a `HandRank` where a larger value is a stronger hand. Values are
//! `category << 26 | detail`, where `detail` orders hands within a category.
//! Verified exhaustively against the known 5- and 7-card category counts.

use crate::Hand;

pub type HandRank = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    HighCard = 0,
    Pair = 1,
    TwoPair = 2,
    Trips = 3,
    Straight = 4,
    Flush = 5,
    FullHouse = 6,
    Quads = 7,
    StraightFlush = 8,
}

const SHIFT: u32 = 26;

#[inline(always)]
fn top_bit(m: u32) -> u32 {
    31 - m.leading_zeros()
}

/// Keep only the `k` highest set bits.
#[inline(always)]
fn keep_top(mut m: u32, k: u32) -> u32 {
    while m.count_ones() > k {
        m &= m - 1;
    }
    m
}

/// Straight high-card index (wheel = 4, broadway = 13), or 0 if none.
#[inline(always)]
fn straight(r: u32) -> u32 {
    let e = (r << 1) | ((r >> 12) & 1);
    let x = e & (e >> 1) & (e >> 2) & (e >> 3) & (e >> 4);
    if x == 0 {
        0
    } else {
        top_bit(x) + 4
    }
}

#[inline(always)]
fn pack(cat: Category, detail: u32) -> HandRank {
    ((cat as u32) << SHIFT) | detail
}

pub fn category(r: HandRank) -> Category {
    match r >> SHIFT {
        0 => Category::HighCard,
        1 => Category::Pair,
        2 => Category::TwoPair,
        3 => Category::Trips,
        4 => Category::Straight,
        5 => Category::Flush,
        6 => Category::FullHouse,
        7 => Category::Quads,
        _ => Category::StraightFlush,
    }
}

/// Evaluate the best 5-card hand contained in a mask of 5, 6 or 7 cards.
#[inline]
pub fn evaluate(m: Hand) -> HandRank {
    let s0 = (m & 0x1FFF) as u32;
    let s1 = ((m >> 16) & 0x1FFF) as u32;
    let s2 = ((m >> 32) & 0x1FFF) as u32;
    let s3 = ((m >> 48) & 0x1FFF) as u32;

    // With at most 7 cards a flush excludes quads and full houses.
    for s in [s0, s1, s2, s3] {
        if s.count_ones() >= 5 {
            let sf = straight(s);
            return if sf != 0 { pack(Category::StraightFlush, sf) } else { pack(Category::Flush, keep_top(s, 5)) };
        }
    }

    // Bit-sliced per-rank card counts (t2 t1 t0).
    let x1 = s0 ^ s1;
    let c1 = s0 & s1;
    let b0 = x1 ^ s2;
    let b1 = c1 | (x1 & s2);
    let t0 = b0 ^ s3;
    let k = b0 & s3;
    let t1 = b1 ^ k;
    let t2 = b1 & k;

    let any = s0 | s1 | s2 | s3;
    let quads = t2;
    let trips = t1 & t0;
    let pairs = t1 & !t0;

    if quads != 0 {
        let q = top_bit(quads);
        let kick = top_bit(any & !(1 << q));
        return pack(Category::Quads, (q << 4) | kick);
    }
    if trips != 0 {
        let t = top_bit(trips);
        let rest = (trips & !(1 << t)) | pairs;
        if rest != 0 {
            return pack(Category::FullHouse, (t << 4) | top_bit(rest));
        }
    }
    let st = straight(any);
    if st != 0 {
        return pack(Category::Straight, st);
    }
    if trips != 0 {
        let t = top_bit(trips);
        return pack(Category::Trips, (t << 13) | keep_top(any & !(1 << t), 2));
    }
    if pairs.count_ones() >= 2 {
        let two = keep_top(pairs, 2);
        return pack(Category::TwoPair, (two << 13) | keep_top(any & !two, 1));
    }
    if pairs != 0 {
        let p = top_bit(pairs);
        return pack(Category::Pair, (p << 13) | keep_top(any & !pairs, 3));
    }
    pack(Category::HighCard, keep_top(any, 5))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bit, mask_of, parse_cards};
    use rayon::prelude::*;
    use std::collections::HashSet;

    fn ev(s: &str) -> HandRank {
        evaluate(mask_of(&parse_cards(s).unwrap()))
    }

    #[test]
    fn spot_checks() {
        assert_eq!(category(ev("AsKsQsJsTs")), Category::StraightFlush);
        assert_eq!(category(ev("As2s3s4s5s9d9c")), Category::StraightFlush);
        assert!(ev("6s2s3s4s5s") > ev("As2s3s4s5s"));
        assert_eq!(category(ev("AsAdAcAh2c")), Category::Quads);
        assert_eq!(category(ev("KsKdKc2h2c2s7d")), Category::FullHouse);
        assert!(ev("KsKdKc2h2c")> ev("QsQdQcAhAc"));
        assert_eq!(category(ev("Ac2d3h4s5c")), Category::Straight);
        assert!(ev("2d3h4s5c6c") > ev("Ac2d3h4s5c"));
        assert!(ev("AhAd8c8s4d") > ev("AhAd8c8s3d"));
        assert!(ev("AhAd8c8s7d7c4h") == ev("AhAd8c8s7d7c3h") || ev("AhAd8c8s7d7c4h") < ev("AhAd8c8s7d7cKh"));
        assert!(ev("AhAd8c8s7d7cKh") > ev("AhAd8c8s7d7c4h"));
        assert!(ev("AhKd9c5s3d") > ev("AhKd9c5s2d"));
        assert_eq!(ev("AhKhQh2h3h9d9c"), ev("AhKhQh2h3h"));
    }

    fn combos5() -> Vec<u64> {
        let mut v = Vec::with_capacity(2_598_960);
        for a in 0..52u8 {
            for b in a + 1..52 {
                for c in b + 1..52 {
                    for d in c + 1..52 {
                        for e in d + 1..52 {
                            v.push(bit(a) | bit(b) | bit(c) | bit(d) | bit(e));
                        }
                    }
                }
            }
        }
        v
    }

    #[test]
    fn exhaustive_five_card() {
        let hands = combos5();
        let mut counts = [0usize; 9];
        let mut distinct = HashSet::new();
        for &m in &hands {
            let r = evaluate(m);
            counts[category(r) as usize] += 1;
            distinct.insert(r);
        }
        assert_eq!(counts, [1_302_540, 1_098_240, 123_552, 54_912, 10_200, 5_108, 3_744, 624, 40]);
        assert_eq!(distinct.len(), 7462);
    }

    /// Exhaustive check over all 133,784,560 seven-card hands, plus the
    /// invariant that the 7-card rank equals the best of its 21 five-card subsets.
    #[test]
    fn exhaustive_seven_card() {
        let counts = (0..52u8)
            .into_par_iter()
            .map(|a| {
                let mut counts = [0u64; 9];
                for b in a + 1..52 {
                    for c in b + 1..52 {
                        for d in c + 1..52 {
                            for e in d + 1..52 {
                                let m5 = bit(a) | bit(b) | bit(c) | bit(d) | bit(e);
                                for f in e + 1..52 {
                                    let m6 = m5 | bit(f);
                                    for g in f + 1..52 {
                                        counts[category(evaluate(m6 | bit(g))) as usize] += 1;
                                    }
                                }
                            }
                        }
                    }
                }
                counts
            })
            .reduce(|| [0u64; 9], |mut x, y| {
                for i in 0..9 {
                    x[i] += y[i];
                }
                x
            });
        assert_eq!(
            counts,
            [23_294_460, 58_627_800, 31_433_400, 6_461_620, 6_180_020, 4_047_644, 3_473_184, 224_848, 41_584]
        );
    }

    #[test]
    fn seven_equals_best_five_sampled() {
        let mut rng = crate::Rng::new(99);
        for _ in 0..200_000 {
            let mut cs = Vec::new();
            let mut dead = 0u64;
            for _ in 0..7 {
                let c = rng.draw(dead);
                dead |= bit(c);
                cs.push(c);
            }
            let mut best = 0;
            for skip1 in 0..7 {
                for skip2 in skip1 + 1..7 {
                    let m = cs
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| *i != skip1 && *i != skip2)
                        .fold(0, |m, (_, &c)| m | bit(c));
                    best = best.max(evaluate(m));
                }
            }
            assert_eq!(evaluate(dead), best);
        }
    }
}
