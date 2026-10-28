//! Belief distributions over the 1326 hole-card pairs, one per seat (supplementary,
//! "Structure of imperfect-information subgames").
//!
//! Each seat's distribution is what an outside observer believes about that seat's private
//! cards: it starts uniform, loses hands that collide with the board (card removal), and is
//! multiplied by the probability the seat's strategy gave to every observed action (Bayes'
//! rule). Our own beliefs about our own range are tracked identically. Marginals are kept
//! independent per seat; the solver samples *joint* deals from them by rejection so card
//! removal between players is exact there.

use cards::{all_holes, bit, Card, Hand};
use std::sync::OnceLock;

pub const N: usize = 1326;

struct Tables {
    holes: Vec<[Card; 2]>,
    masks: Vec<Hand>,
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let holes = all_holes();
        let masks = holes.iter().map(|h| bit(h[0]) | bit(h[1])).collect();
        Tables { holes, masks }
    })
}

/// All 1326 hole pairs in `cards::hole_index` order.
pub fn holes() -> &'static [[Card; 2]] {
    &tables().holes
}
/// Card mask of hole pair `i`.
#[inline]
pub fn hole_mask(i: usize) -> Hand {
    tables().masks[i]
}

#[derive(Clone, Debug)]
pub struct Beliefs {
    /// `w[seat][hole index]`, each row sums to 1 (or to 0 for an unused seat).
    pub w: Vec<Vec<f32>>,
}

impl Beliefs {
    pub fn uniform(players: usize) -> Beliefs {
        Beliefs {
            w: (0..players).map(|_| vec![1.0 / N as f32; N]).collect(),
        }
    }

    pub fn players(&self) -> usize {
        self.w.len()
    }

    /// Zero every hand that shares a card with `dead` and renormalise all rows.
    pub fn remove_cards(&mut self, dead: Hand) {
        let t = tables();
        for row in &mut self.w {
            for (i, x) in row.iter_mut().enumerate() {
                if t.masks[i] & dead != 0 {
                    *x = 0.0;
                }
            }
            normalize(row);
        }
    }

    /// Bayes update of `seat`: multiply by `lik(hole index)` and renormalise. If the observed
    /// action had zero probability under every hand with mass (the observer was "surprised"),
    /// the row is left unchanged. Returns whether the update was applied.
    pub fn bayes(&mut self, seat: usize, mut lik: impl FnMut(usize) -> f32) -> bool {
        let row = &mut self.w[seat];
        let old = row.clone();
        for (i, x) in row.iter_mut().enumerate() {
            if *x > 0.0 {
                *x *= lik(i).max(0.0);
            }
        }
        if !normalize(row) {
            *row = old;
            return false;
        }
        true
    }

    /// Cumulative distribution of `seat` (for sampling by binary search).
    pub fn cdf(&self, seat: usize) -> Vec<f64> {
        let mut acc = 0.0f64;
        self.w[seat]
            .iter()
            .map(|&x| {
                acc += x as f64;
                acc
            })
            .collect()
    }

    /// Make sure hand `i` of `seat` has positive mass (our own actual hand must stay
    /// searchable even if the average strategy gave its actions probability 0).
    pub fn floor(&mut self, seat: usize, i: usize, eps: f32) {
        if self.w[seat][i] < eps {
            self.w[seat][i] = eps;
            normalize(&mut self.w[seat]);
        }
    }
}

fn normalize(row: &mut [f32]) -> bool {
    let s: f64 = row.iter().map(|&x| x as f64).sum();
    if s <= 0.0 {
        return false;
    }
    let inv = (1.0 / s) as f32;
    row.iter_mut().for_each(|x| *x *= inv);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use cards::{hole_index, mask_of, parse_cards};

    #[test]
    fn uniform_sums_to_one_and_removal_works() {
        let mut b = Beliefs::uniform(3);
        for r in &b.w {
            assert!((r.iter().map(|&x| x as f64).sum::<f64>() - 1.0).abs() < 1e-5);
        }
        let board = mask_of(&parse_cards("AsKdQh").unwrap());
        b.remove_cards(board);
        for r in &b.w {
            let live = (0..N).filter(|&i| hole_mask(i) & board == 0).count();
            assert_eq!(live, 1176); // C(49,2)
            assert!((r.iter().map(|&x| x as f64).sum::<f64>() - 1.0).abs() < 1e-5);
            for i in 0..N {
                assert_eq!(r[i] > 0.0, hole_mask(i) & board == 0);
            }
        }
    }

    /// Bayes against a brute-force enumeration with exact rational-style accumulation in f64.
    #[test]
    fn bayes_matches_brute_force() {
        let mut b = Beliefs::uniform(2);
        let board = mask_of(&parse_cards("2c7d9h").unwrap());
        b.remove_cards(board);
        // Strategy: raise with prob depending on the high card; two sequential observations.
        let p_raise = |i: usize| 0.05 + 0.9 * (holes()[i][0].max(holes()[i][1]) as f32 / 51.0);
        let p_call = |i: usize| 1.0 - p_raise(i);
        b.bayes(0, p_raise);
        b.bayes(0, p_call);
        let mut brute = vec![0f64; N];
        for i in 0..N {
            if hole_mask(i) & board == 0 {
                brute[i] = p_raise(i) as f64 * p_call(i) as f64;
            }
        }
        let z: f64 = brute.iter().sum();
        for i in 0..N {
            assert!((b.w[0][i] as f64 - brute[i] / z).abs() < 1e-6, "hand {i}");
        }
        assert!((b.w[0].iter().map(|&x| x as f64).sum::<f64>() - 1.0).abs() < 1e-5);
        // Other seat untouched.
        assert_eq!(b.w[1][hole_index(10, 11)], b.w[1][hole_index(20, 31)]);
    }

    #[test]
    fn surprise_leaves_row_unchanged() {
        let mut b = Beliefs::uniform(2);
        let before = b.w[1].clone();
        assert!(!b.bayes(1, |_| 0.0));
        assert_eq!(b.w[1], before);
    }
}
