//! Subgame tree rules: search action abstraction, off-tree action insertion, public-path
//! hashing and the paper's depth-limit placement.
//!
//! The subgame is not materialised: it is the real betting tree of the round (and, when the
//! subgame goes to the end of the game, of the later rounds) generated on the fly from a
//! `State`. A public node is identified by the hash of the *real* actions taken since the
//! subgame root (`step_path`), so an off-tree action added at a node (`Extras`) simply extends
//! that node's action list (supplementary Algorithm 2, `AddAction`).

use nlhe::abstraction::{abstract_actions, ActionList, BetSizing, Menu};
use nlhe::{Action, State, Street};
use std::collections::HashMap;

/// Off-tree raise amounts added per public node (path hash).
pub type Extras = HashMap<u64, Vec<i32>>;

#[inline]
pub fn code(a: Action) -> u64 {
    match a {
        Action::Fold => 1,
        Action::Check | Action::Call => 2,
        Action::RaiseTo(t) => 3 + ((t as u64) << 2),
    }
}

/// Extend a public path hash by one real action.
#[inline]
pub fn step_path(path: u64, a: Action) -> u64 {
    let mut z = (path.rotate_left(17) ^ code(a)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z ^= z >> 29;
    z.wrapping_mul(0xBF58_476D_1CE4_E5B9) | 1
}

/// Default search action abstraction: first raise {0.5, 1, 2} x pot + all-in, second raise
/// {1} + all-in, deeper only all-in (the paper uses 1-6 sizes, typically at most five);
/// at most 4 raises per street.
pub fn default_sizing() -> BetSizing {
    let m = Menu::new;
    let d = [m(&[0.5, 1.0, 2.0], true), m(&[1.0], true), m(&[], true)];
    BetSizing::new([&d, &d, &d, &d], [4, 4, 4, 4])
}

/// Abstraction for rounds after the subgame's root round: first raise {1 x pot} + all-in,
/// then only all-in; at most 3 raises per street.
pub fn later_sizing() -> BetSizing {
    let m = Menu::new;
    let d = [m(&[1.0], true), m(&[], true)];
    BetSizing::new([&d, &d, &d, &d], [3, 3, 3, 3])
}

/// Small abstraction for fast tests / large subgames: {1 x pot, all-in}.
pub fn small_sizing() -> BetSizing {
    let m = Menu::new;
    let d = [m(&[1.0], true), m(&[], true)];
    BetSizing::new([&d, &d, &d, &d], [3, 3, 3, 3])
}

/// Search actions at `st`: the sizing's abstract actions plus every legal off-tree raise in
/// `extras`, ascending, all-in last. Extras bypass the per-street raise cap.
pub fn search_actions(st: &State, sizing: &BetSizing, extras: &[i32], out: &mut ActionList) {
    abstract_actions(st, sizing, out);
    if extras.is_empty() {
        return;
    }
    let l = st.legal();
    if !l.can_raise {
        return;
    }
    let mut raises: Vec<i32> = out
        .as_slice()
        .iter()
        .filter_map(|a| if let Action::RaiseTo(t) = a { Some(*t) } else { None })
        .collect();
    let before = raises.len();
    for &t in extras {
        if t >= l.min_raise_to && t <= l.max_raise_to && !raises.contains(&t) {
            raises.push(t);
        }
    }
    if raises.len() == before {
        return;
    }
    raises.sort_unstable();
    let keep: Vec<Action> = out.as_slice().iter().copied().filter(|a| !matches!(a, Action::RaiseTo(_))).collect();
    out.clear();
    for a in keep {
        out.push(a);
    }
    for t in raises {
        out.push(Action::RaiseTo(t));
    }
}

/// Where the subgame stops searching and hands over to continuation-strategy leaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    /// Solve to the end of the hand (exact showdown payoffs).
    EndOfGame,
    /// Leaves at the chance node that opens the next betting round (preflop search).
    EndOfRound,
    /// Leaves at the start of the next round or immediately after the second raise of the
    /// round, whichever comes first (flop with > 2 players at the start of the round).
    SecondRaise,
}

/// The paper's depth rule for a subgame rooted at the start of a betting round.
/// `root` is the state at the start of the round (players still in the hand = not folded).
pub fn limit_for(root: &State) -> Limit {
    match root.street() {
        Street::Preflop => Limit::EndOfRound,
        Street::Flop if root.live_count() > 2 => Limit::SecondRaise,
        _ => Limit::EndOfGame,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nlhe::{GameConfig, Step};

    fn flop_root(players: usize, folds: usize) -> State {
        let cfg = GameConfig::new(players);
        let mut s = State::new(&cfg);
        let mut guard = 0;
        let mut folded = 0;
        loop {
            guard += 1;
            assert!(guard < 50);
            let l = s.legal();
            let a = if l.can_fold && folded < folds {
                folded += 1;
                Action::Fold
            } else if l.can_check {
                Action::Check
            } else {
                Action::Call
            };
            if let Step::NewStreet(Street::Flop) = s.apply(a) {
                return s;
            }
        }
    }

    #[test]
    fn depth_limit_rules() {
        assert_eq!(limit_for(&State::new(&GameConfig::new(6))), Limit::EndOfRound);
        assert_eq!(limit_for(&flop_root(6, 3)), Limit::SecondRaise); // 3 players
        assert_eq!(limit_for(&flop_root(3, 0)), Limit::SecondRaise);
        assert_eq!(limit_for(&flop_root(6, 4)), Limit::EndOfGame); // heads-up on the flop
        assert_eq!(limit_for(&flop_root(2, 0)), Limit::EndOfGame);
        // Turn/river always to the end of the game, even multiway.
        let mut s = flop_root(4, 0);
        loop {
            let l = s.legal();
            let a = if l.can_check { Action::Check } else { Action::Call };
            if let Step::NewStreet(Street::Turn) = s.apply(a) {
                break;
            }
        }
        assert_eq!(s.live_count(), 4);
        assert_eq!(limit_for(&s), Limit::EndOfGame);
    }

    #[test]
    fn extras_are_inserted_sorted_and_legal() {
        let s = flop_root(2, 0);
        let sz = default_sizing();
        let mut base = ActionList::new();
        search_actions(&s, &sz, &[], &mut base);
        let l = s.legal();
        let odd = l.min_raise_to + 777;
        let mut with = ActionList::new();
        search_actions(&s, &sz, &[odd, odd, l.max_raise_to + 5, l.min_raise_to - 1], &mut with);
        assert_eq!(with.len(), base.len() + 1);
        assert!(with.as_slice().contains(&Action::RaiseTo(odd)));
        let r: Vec<i32> = with.as_slice().iter().filter_map(|a| if let Action::RaiseTo(t) = a { Some(*t) } else { None }).collect();
        assert!(r.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(*r.last().unwrap(), l.max_raise_to);
    }

    #[test]
    fn random_walks_are_legal() {
        let sz = default_sizing();
        let mut rng = cards::Rng::new(3);
        for _ in 0..500 {
            let mut s = State::new(&GameConfig::new(2 + rng.below(5) as usize));
            while !s.is_terminal() {
                let mut l = ActionList::new();
                search_actions(&s, &sz, &[], &mut l);
                let lg = s.legal();
                for a in l.as_slice() {
                    match *a {
                        Action::Fold => assert!(lg.can_fold),
                        Action::Check => assert!(lg.can_check),
                        Action::Call => assert!(!lg.can_check),
                        Action::RaiseTo(t) => assert!(lg.can_raise && t >= lg.min_raise_to && t <= lg.max_raise_to),
                    }
                }
                s.apply(l.get(rng.below(l.len() as u32) as usize));
            }
        }
    }
}
