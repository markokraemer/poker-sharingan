//! Toy river with a closed-form answer, used to validate the solver.
//!
//! Board `Ts Tc Th Td 2c` (quads; only the kicker matters). OOP holds half "nuts" (any `AA`,
//! kicker A) and half air (`33`, kicker 3); IP holds only bluff catchers (`KK`, kicker K).
//! Pot 200 (100 each), one pot-sized bet, no raises. Analytic equilibrium (bet = pot):
//! * OOP value always bets; air bets with probability 1/2 (so bluffs are 1/3 of bets);
//! * IP calls with probability P/(P+B) = 1/2;
//! * value to OOP = 50 chips (0.25 pot), IP = -50.

use crate::replay::Replay;
use crate::solver::{Game, N};
use cards::{all_holes, parse_cards};
use nlhe::abstraction::{BetSizing, Menu};
use nlhe::Action;

pub fn toy_game() -> Game {
    let board: [u8; 5] = parse_cards("TsTcThTd2c").unwrap().try_into().unwrap();
    let mut r = Replay::new(2, &board);
    // Heads-up: SB (seat 0) limps, BB checks, then check-check on every street.
    r.apply(Action::Call).unwrap();
    r.apply(Action::Check).unwrap();
    for _ in 0..2 {
        r.apply(Action::Check).unwrap();
        r.apply(Action::Check).unwrap();
    }
    assert_eq!(r.state.street(), nlhe::Street::River);
    let holes = all_holes();
    let pair_of = |rank: u8| move |h: &[u8; 2]| h[0] >> 2 == rank && h[1] >> 2 == rank;
    let (aa, tt, kk) = (pair_of(12), pair_of(1), pair_of(11));
    let (mut oop, mut ip) = (vec![0.0; N], vec![0.0; N]);
    for (i, h) in holes.iter().enumerate() {
        if aa(h) || tt(h) {
            oop[i] = 1.0;
        }
        if kk(h) {
            ip[i] = 1.0;
        }
    }
    let menu = [Menu::new(&[1.0], false)];
    let sizing = BetSizing::new([&menu, &menu, &menu, &menu], [1, 1, 1, 1]);
    Game::new(r.state, [1, 0], board, [oop, ip], &sizing, 100).unwrap()
}
