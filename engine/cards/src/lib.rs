//! Card primitives shared by every crate in the workspace.
//!
//! Encoding:
//! - `Card` is a `u8` in `0..52`: `rank * 4 + suit`, rank `0..13` = 2..A, suit `0..4` = c,d,h,s.
//! - `Hand` is a `u64` bitmask in *suit-major* layout: card bit = `suit * 16 + rank`.
//!   This makes per-suit 13-bit rank masks a shift + mask, which the evaluator relies on.

pub mod eval;
pub mod rng;

pub use eval::{category, evaluate, Category, HandRank};
pub use rng::Rng;

pub type Card = u8;
pub type Hand = u64;

pub const NUM_CARDS: usize = 52;
pub const NUM_RANKS: u8 = 13;
pub const NUM_SUITS: u8 = 4;
pub const RANK_CHARS: &[u8; 13] = b"23456789TJQKA";
pub const SUIT_CHARS: &[u8; 4] = b"cdhs";
pub const FULL_DECK: Hand = 0x1FFF_1FFF_1FFF_1FFF;

#[inline(always)]
pub const fn card(rank: u8, suit: u8) -> Card {
    rank * 4 + suit
}

#[inline(always)]
pub const fn rank_of(c: Card) -> u8 {
    c >> 2
}

#[inline(always)]
pub const fn suit_of(c: Card) -> u8 {
    c & 3
}

#[inline(always)]
pub const fn bit(c: Card) -> Hand {
    1u64 << ((c & 3) as u32 * 16 + (c >> 2) as u32)
}

#[inline]
pub fn mask_of(cards: &[Card]) -> Hand {
    cards.iter().fold(0, |m, &c| m | bit(c))
}

/// Inverse of `bit` for a single-bit mask.
#[inline(always)]
pub fn card_from_bit_index(idx: u32) -> Card {
    let suit = (idx / 16) as u8;
    let rank = (idx % 16) as u8;
    card(rank, suit)
}

/// Iterate the cards of a mask in increasing bit order.
#[inline]
pub fn cards_in(mut m: Hand) -> impl Iterator<Item = Card> {
    std::iter::from_fn(move || {
        if m == 0 {
            None
        } else {
            let i = m.trailing_zeros();
            m &= m - 1;
            Some(card_from_bit_index(i))
        }
    })
}

pub fn parse_card(s: &str) -> Option<Card> {
    let b = s.as_bytes();
    if b.len() != 2 {
        return None;
    }
    let r = RANK_CHARS.iter().position(|&c| c == b[0].to_ascii_uppercase())? as u8;
    let su = SUIT_CHARS.iter().position(|&c| c == b[1].to_ascii_lowercase())? as u8;
    Some(card(r, su))
}

/// Parse a run of cards like `"AsKd"` or `"As Kd 7h"`.
pub fn parse_cards(s: &str) -> Option<Vec<Card>> {
    let compact: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.len() % 2 != 0 {
        return None;
    }
    compact
        .as_bytes()
        .chunks(2)
        .map(|ch| parse_card(std::str::from_utf8(ch).ok()?))
        .collect()
}

pub fn card_str(c: Card) -> String {
    let mut s = String::with_capacity(2);
    s.push(RANK_CHARS[rank_of(c) as usize] as char);
    s.push(SUIT_CHARS[suit_of(c) as usize] as char);
    s
}

pub fn cards_str(cs: &[Card]) -> String {
    cs.iter().map(|&c| card_str(c)).collect()
}

/// Index of an unordered hole-card pair in `0..1326` (colex order over card ids).
#[inline(always)]
pub fn hole_index(a: Card, b: Card) -> usize {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    (hi as usize) * (hi as usize - 1) / 2 + lo as usize
}

/// All 1326 hole pairs in `hole_index` order.
pub fn all_holes() -> Vec<[Card; 2]> {
    let mut v = Vec::with_capacity(1326);
    for hi in 1..52u8 {
        for lo in 0..hi {
            v.push([lo, hi]);
        }
    }
    v
}

/// Lossless preflop class in `0..169`: pairs 0..13, suited 13..91, offsuit 91..169.
pub fn preflop_class(a: Card, b: Card) -> usize {
    let (r1, r2) = (rank_of(a).max(rank_of(b)) as usize, rank_of(a).min(rank_of(b)) as usize);
    if r1 == r2 {
        r1
    } else {
        let tri = r1 * (r1 - 1) / 2 + r2; // 0..78
        if suit_of(a) == suit_of(b) {
            13 + tri
        } else {
            91 + tri
        }
    }
}

pub fn preflop_class_name(class: usize) -> String {
    let r = |i: usize| RANK_CHARS[i] as char;
    if class < 13 {
        format!("{}{}", r(class), r(class))
    } else {
        let (tri, suffix) = if class < 91 { (class - 13, 's') } else { (class - 91, 'o') };
        let mut r1 = 1;
        while (r1 + 1) * r1 / 2 <= tri {
            r1 += 1;
        }
        let r2 = tri - r1 * (r1 - 1) / 2;
        format!("{}{}{}", r(r1), r(r2), suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_cards() {
        for c in 0..52u8 {
            assert_eq!(parse_card(&card_str(c)), Some(c));
            assert_eq!(cards_in(bit(c)).collect::<Vec<_>>(), vec![c]);
        }
        assert_eq!(FULL_DECK.count_ones(), 52);
        assert_eq!(mask_of(&(0..52).collect::<Vec<_>>()), FULL_DECK);
    }

    #[test]
    fn hole_index_dense() {
        let holes = all_holes();
        assert_eq!(holes.len(), 1326);
        for (i, h) in holes.iter().enumerate() {
            assert_eq!(hole_index(h[0], h[1]), i);
            assert_eq!(hole_index(h[1], h[0]), i);
        }
    }

    #[test]
    fn preflop_classes() {
        let mut counts = [0usize; 169];
        for h in all_holes() {
            counts[preflop_class(h[0], h[1])] += 1;
        }
        for (c, &n) in counts.iter().enumerate() {
            let expect = if c < 13 { 6 } else if c < 91 { 4 } else { 12 };
            assert_eq!(n, expect, "class {}", preflop_class_name(c));
        }
        assert_eq!(preflop_class_name(12), "AA");
        assert_eq!(preflop_class_name(preflop_class(parse_card("As").unwrap(), parse_card("Ks").unwrap())), "AKs");
        assert_eq!(preflop_class_name(preflop_class(parse_card("7d").unwrap(), parse_card("2c").unwrap())), "72o");
    }
}
