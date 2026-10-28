//! Deterministic dealing (TABLE_API.md section 7).
//!
//! `deck_i = Fisher-Yates(ChaCha20(key = HMAC_SHA256(seed_utf8, "deal:" + i)))` over the cards in the
//! order `2c 2d 2h 2s 3c ... As` (the `cards` crate ids 0..52). The shuffle runs `i = 51 .. 1` and draws
//! `j in [0, i]` by rejection sampling on consecutive little-endian `u32` words of the ChaCha20
//! keystream (the stream with an all-zero nonce, block counter 0). Dealing order from the deck:
//! hole cards one at a time clockwise starting left of the button (two rounds), then burn+3 flop,
//! burn+turn, burn+river.

use crate::util::{hmac_sha256, sha256_hex};
use rand_chacha::rand_core::{RngCore, SeedableRng};
use rand_chacha::ChaCha20Rng;

pub fn deal_deck(seed: &str, deal_no: u64) -> Vec<u8> {
    let key = hmac_sha256(seed.as_bytes(), format!("deal:{deal_no}").as_bytes());
    let mut rng = ChaCha20Rng::from_seed(key.try_into().expect("32-byte hmac"));
    let mut deck: Vec<u8> = (0..52).collect();
    for i in (1..52usize).rev() {
        let n = (i + 1) as u32;
        let zone = u32::MAX - (u32::MAX % n); // accept x < zone
        let j = loop {
            let x = rng.next_u32();
            if x < zone {
                break (x % n) as usize;
            }
        };
        deck.swap(i, j);
    }
    deck
}

/// Put `prefix` first and keep the remaining cards in their `base` order (sandbox scripted decks).
pub fn with_prefix(base: &[u8], prefix: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = prefix.to_vec();
    out.extend(base.iter().copied().filter(|c| !prefix.contains(c)));
    out
}

pub fn commitment(deck: &[u8], salt: &str) -> String {
    let mut s = cards::cards_str(deck);
    s.push_str(salt);
    sha256_hex(s.as_bytes())
}
