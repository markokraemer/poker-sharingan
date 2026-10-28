//! Open Poker seats -> nlhe seats.
//!
//! nlhe convention: seat 0 = small blind, 1 = big blind, 2.. follow, the last seat is the
//! button; heads-up the button is seat 0 (SB). Open Poker numbers seats clockwise and sends
//! `dealer_seat`; the SB is the next dealt-in seat after the dealer, the BB the next after
//! that (verified on recorded hands: dealer 0 -> SB 1, BB 2, first to act 3).
//!
//! **Heads-up (observed live):** Open Poker applies the same rule with two players, so the
//! *dealer is the BB* and the other player is the SB (first to act preflop, second postflop).
//! The roles are standard; only the button-to-role assignment differs from the usual
//! "button = SB". The layout therefore has no heads-up special case.
//!
//! # Fewer than six players
//! - `embed6 = true` (blueprint policies, whose infoset keys encode a 6-player action
//!   history): the hand is embedded in the 6-max game. The first `6 - n` non-blind seats
//!   (UTG, HJ, ...) are phantom players who fold immediately, so real players keep their true
//!   relative order (SB, BB, then the rest, button last) and the blueprint sees a history that
//!   "folded around" - exactly what happens in a 6-max game where early positions fold.
//!   Valid for 3..=5 players. Heads-up cannot be embedded (the HU button is SB and acts *last*
//!   postflop, which no 6-max seat order reproduces), so n = 2 always uses a native 2-player
//!   engine and the session decides which policy plays it (see `session::HuMode`).
//! - `embed6 = false` (stateless heuristics like the built-ins): native n-player engine.

use crate::SCALE;
use nlhe::{GameConfig, MAX_PLAYERS};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct Layout {
    pub n_real: usize,
    pub embed6: bool,
    /// Number of phantom seats that fold first (engine seats `2..2+prefold`).
    pub prefold: usize,
    /// engine seat -> Open Poker seat (`None` for phantoms).
    pub op_of_nl: Vec<Option<usize>>,
    pub nl_of_op: BTreeMap<usize, usize>,
    pub cfg: GameConfig,
}

pub fn scale(x: i64) -> i32 {
    (x * SCALE as i64).clamp(0, i32::MAX as i64 / 2) as i32
}

pub fn unscale(x: i32) -> i64 {
    ((x as f64) / SCALE as f64).round() as i64
}

/// Dealt-in seats in engine order `[SB, BB, ..., BTN]`: the SB is the first seat after the dealer.
pub fn engine_order(participants: &BTreeSet<usize>, dealer: usize) -> Vec<usize> {
    let seats: Vec<usize> = participants.iter().copied().collect();
    let n = seats.len();
    let start = seats.iter().position(|&s| s > dealer).unwrap_or(0);
    (0..n).map(|i| seats[(start + i) % n]).collect()
}

impl Layout {
    /// `stacks` are start-of-hand stacks (behind the blinds) in Open Poker chips.
    pub fn build(
        participants: &BTreeSet<usize>,
        dealer: usize,
        stacks: &BTreeMap<usize, i64>,
        sb: i64,
        bb: i64,
        embed6: bool,
    ) -> Result<Layout, String> {
        let n = participants.len();
        if !(2..=MAX_PLAYERS).contains(&n) {
            return Err(format!("{n} dealt-in players (need 2..=6)"));
        }
        // Engine order [SB, BB, ..., BTN] in Open Poker seats.
        let order = engine_order(participants, dealer);
        let embed = embed6 && (3..=5).contains(&n);
        let k = if embed { MAX_PLAYERS - n } else { 0 };
        let players = if embed { MAX_PLAYERS } else { n };
        let mut op_of_nl: Vec<Option<usize>> = vec![None; players];
        for (i, &s) in order.iter().enumerate() {
            let nl = if i < 2 { i } else { i + k };
            op_of_nl[nl] = Some(s);
        }
        let nl_of_op = op_of_nl
            .iter()
            .enumerate()
            .filter_map(|(nl, s)| s.map(|s| (s, nl)))
            .collect();
        let mut st = [0i32; MAX_PLAYERS];
        for (nl, s) in op_of_nl.iter().enumerate() {
            st[nl] = match s {
                Some(s) => scale(*stacks.get(s).ok_or_else(|| format!("no stack for seat {s}"))?),
                None => scale(bb) * 100, // phantom: folds immediately, size irrelevant
            };
        }
        let cfg = GameConfig {
            players,
            stack: 0,
            small_blind: scale(sb),
            big_blind: scale(bb),
            stacks: Some(st),
        };
        Ok(Layout {
            n_real: n,
            embed6: embed,
            prefold: k,
            op_of_nl,
            nl_of_op,
            cfg,
        })
    }

    pub fn nl(&self, op_seat: usize) -> Option<usize> {
        self.nl_of_op.get(&op_seat).copied()
    }
    pub fn op(&self, nl_seat: usize) -> Option<usize> {
        self.op_of_nl.get(nl_seat).copied().flatten()
    }
    /// Position name of an engine seat in an n-handed game (for traces/reports).
    pub fn position_name(&self, op_seat: usize) -> &'static str {
        let Some(nl) = self.nl(op_seat) else { return "?" };
        // index within real players in engine order
        let idx = (0..=nl).filter(|&i| self.op_of_nl[i].is_some()).count() - 1;
        position_name(self.n_real, idx)
    }
}

/// `idx` = index in engine order (0 = SB, 1 = BB, ..., n-1 = BTN).
pub fn position_name(n: usize, idx: usize) -> &'static str {
    if n == 2 {
        return ["BTN/SB", "BB"][idx.min(1)];
    }
    if idx == 0 {
        return "SB";
    }
    if idx == 1 {
        return "BB";
    }
    if idx == n - 1 {
        return "BTN";
    }
    // Positions between BB and BTN: closest to BTN is CO, then HJ, then UTG.
    match n - 1 - idx {
        1 => "CO",
        2 => "HJ",
        _ => "UTG",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(v: &[usize]) -> BTreeSet<usize> {
        v.iter().copied().collect()
    }
    fn stacks(v: &[usize]) -> BTreeMap<usize, i64> {
        v.iter().map(|&s| (s, 2000 + s as i64)).collect()
    }

    #[test]
    fn six_max_rotation_matches_recorded_history() {
        // dealer 0: SB seat 1, BB seat 2, UTG seat 3, ..., BTN seat 0.
        let l = Layout::build(&parts(&[0, 1, 2, 3, 4, 5]), 0, &stacks(&[0, 1, 2, 3, 4, 5]), 10, 20, true).unwrap();
        assert_eq!(l.op_of_nl, vec![Some(1), Some(2), Some(3), Some(4), Some(5), Some(0)]);
        assert_eq!(l.cfg.small_blind, 50);
        assert_eq!(l.cfg.stacks.unwrap()[0], 2001 * 5);
        assert_eq!(l.position_name(3), "UTG");
        assert_eq!(l.position_name(0), "BTN");
        // dealer 5 wraps
        let l = Layout::build(&parts(&[0, 1, 2, 3, 4, 5]), 5, &stacks(&[0, 1, 2, 3, 4, 5]), 10, 20, true).unwrap();
        assert_eq!(l.nl(0), Some(0)); // SB wraps to seat 0
        assert_eq!(l.nl(5), Some(5)); // dealer 5 is the button
    }

    #[test]
    fn short_handed_embedding_keeps_real_order() {
        // 4 players at seats 0,2,3,5, dealer 2: SB 3, BB 5, X 0, BTN 2.
        let l = Layout::build(&parts(&[0, 2, 3, 5]), 2, &stacks(&[0, 2, 3, 5]), 10, 20, true).unwrap();
        assert!(l.embed6 && l.prefold == 2 && l.cfg.players == 6);
        assert_eq!(l.op_of_nl, vec![Some(3), Some(5), None, None, Some(0), Some(2)]);
        assert_eq!(l.position_name(0), "CO");
        // native
        let l = Layout::build(&parts(&[0, 2, 3, 5]), 2, &stacks(&[0, 2, 3, 5]), 10, 20, false).unwrap();
        assert!(!l.embed6 && l.cfg.players == 4);
        assert_eq!(l.op_of_nl, vec![Some(3), Some(5), Some(0), Some(2)]);
        // 3 players: all three early seats are phantoms, BTN is engine seat 5.
        let l = Layout::build(&parts(&[1, 2, 4]), 4, &stacks(&[1, 2, 4]), 10, 20, true).unwrap();
        assert_eq!(l.prefold, 3);
        assert_eq!(l.op_of_nl, vec![Some(1), Some(2), None, None, None, Some(4)]);
    }

    #[test]
    fn heads_up_follows_open_poker_dealer_is_bb_and_is_never_embedded() {
        // Recorded live: seats 1 and 2, dealer 2 -> seat 1 posts SB and acts first preflop.
        let l = Layout::build(&parts(&[1, 2]), 2, &stacks(&[1, 2]), 10, 20, true).unwrap();
        assert!(!l.embed6);
        assert_eq!(l.op_of_nl, vec![Some(1), Some(2)]);
        assert_eq!(l.position_name(1), "BTN/SB");
        assert_eq!(l.position_name(2), "BB");
        let l = Layout::build(&parts(&[2, 4]), 3, &stacks(&[2, 4]), 10, 20, true).unwrap();
        assert_eq!(l.op_of_nl, vec![Some(4), Some(2)]);
    }

    #[test]
    fn rejects_bad_counts() {
        assert!(Layout::build(&parts(&[1]), 0, &stacks(&[1]), 10, 20, true).is_err());
        assert_eq!(unscale(scale(137)), 137);
    }
}
