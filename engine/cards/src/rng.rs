//! Small, fast, seedable RNG (xoshiro256++). Deterministic across platforms.

use crate::{card_from_bit_index, Card, Hand};

#[derive(Clone, Debug)]
pub struct Rng {
    s: [u64; 4],
}

#[inline(always)]
fn splitmix(x: &mut u64) -> u64 {
    *x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut x = seed;
        Rng { s: [splitmix(&mut x), splitmix(&mut x), splitmix(&mut x), splitmix(&mut x)] }
    }

    #[inline(always)]
    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.s;
        let result = s[0].wrapping_add(s[3]).rotate_left(23).wrapping_add(s[0]);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }

    /// Uniform in `[0, 1)`.
    #[inline(always)]
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    #[inline(always)]
    pub fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 * (1.0 / (1u32 << 24) as f32)
    }

    /// Uniform in `0..n` (Lemire's multiply-shift, negligible bias for small n).
    #[inline(always)]
    pub fn below(&mut self, n: u32) -> u32 {
        (((self.next_u64() >> 32) * n as u64) >> 32) as u32
    }

    /// Sample an index from unnormalised non-negative weights.
    #[inline]
    pub fn sample_weighted(&mut self, weights: &[f32]) -> usize {
        let total: f32 = weights.iter().sum();
        if total <= 0.0 {
            return self.below(weights.len() as u32) as usize;
        }
        let mut x = self.f32() * total;
        for (i, &w) in weights.iter().enumerate() {
            if x < w {
                return i;
            }
            x -= w;
        }
        weights.len() - 1
    }

    /// Draw one card not in `dead`, uniformly. `dead` must leave at least one card.
    #[inline]
    pub fn draw(&mut self, dead: Hand) -> Card {
        let live = crate::FULL_DECK & !dead;
        let n = live.count_ones();
        let mut k = self.below(n);
        let mut m = live;
        loop {
            let i = m.trailing_zeros();
            if k == 0 {
                return card_from_bit_index(i);
            }
            k -= 1;
            m &= m - 1;
        }
    }

    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.below(i as u32 + 1) as usize;
            v.swap(i, j);
        }
    }

    /// A freshly shuffled 52-card deck.
    pub fn deck(&mut self) -> [Card; 52] {
        let mut d = [0u8; 52];
        for (i, c) in d.iter_mut().enumerate() {
            *c = i as u8;
        }
        self.shuffle(&mut d);
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_uniformish() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        assert_eq!(a.next_u64(), b.next_u64());
        let mut counts = [0u32; 52];
        let mut r = Rng::new(1);
        for _ in 0..520_000 {
            counts[r.draw(0) as usize] += 1;
        }
        for &c in &counts {
            assert!((9_500..10_500).contains(&c), "{c}");
        }
    }

    #[test]
    fn draw_respects_dead() {
        let mut r = Rng::new(3);
        let dead = crate::FULL_DECK & !crate::bit(17) & !crate::bit(40);
        for _ in 0..100 {
            let c = r.draw(dead);
            assert!(c == 17 || c == 40);
        }
    }
}
