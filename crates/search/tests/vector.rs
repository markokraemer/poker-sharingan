//! Vector-form Linear CFR: correctness against the referee's exact river solver / best response.
use cards::{parse_cards, Card};
use nlhe::abstraction::{BetSizing, Menu};
use nlhe::{Action, GameConfig, State, Step, Street};
use referee::solver::{Game, Strategy};
use search::tree::{default_sizing, later_sizing, step_path, Extras};
use search::vector::{exploitability, solve_vec, VecGame, VecParams};
use std::collections::HashMap;

fn cs(s: &str) -> Vec<Card> {
    parse_cards(s).unwrap()
}

/// HU: limp, check, then bet/call to reach `street` with a decent pot.
fn root_at(street: Street) -> State {
    let mut s = State::new(&GameConfig::new(2));
    let mut guard = 0;
    loop {
        guard += 1;
        assert!(guard < 40);
        let l = s.legal();
        let a = if l.can_raise && s.street() == Street::Preflop && s.raises_this_street() == 0 {
            Action::RaiseTo(l.min_raise_to * 3)
        } else if l.can_check {
            Action::Check
        } else {
            Action::Call
        };
        if let Step::NewStreet(st) = s.apply(a) {
            if st == street {
                return s;
            }
        }
    }
}

fn ranges(board: &[Card], seed: u64) -> [Vec<f64>; 2] {
    let mut rng = cards::Rng::new(seed);
    let bm = cards::mask_of(board);
    let hl = cards::all_holes();
    [0, 1].map(|_| {
        hl.iter()
            .map(|h| if (cards::bit(h[0]) | cards::bit(h[1])) & bm != 0 { 0.0 } else { 0.05 + rng.f64() * (rng.f64() < 0.5) as u8 as f64 })
            .collect()
    })
}

fn small() -> BetSizing {
    let m = Menu::new;
    let d = [m(&[0.5, 1.0], true), m(&[1.0], true)];
    BetSizing::new([&d, &d, &d, &d], [3, 3, 3, 3])
}

#[test]
fn river_vector_cfr_plus_equals_referee_and_my_br_equals_referee_br() {
    let root = root_at(Street::River);
    let board = cs("2c7d9hJsKc");
    let r = ranges(&board, 5);
    let (sz, later) = (small(), later_sizing());
    let none = Extras::new();
    let frozen = HashMap::new();
    let g = VecGame { root, board: &board, seats: [1, 0], ranges: r.clone(), sizing: &sz, later: &later, extras: &none, me: None, my_hole: None, frozen: &frozen, pooled: None };
    let b5: [Card; 5] = board.clone().try_into().unwrap();
    let rg = Game::new(root, [1, 0], b5, r.clone(), &sz, 100).unwrap();
    let vp = VecParams { iters: 300, plus: true, keep_all: true, ..Default::default() };
    let sol = solve_vec(&g, &vp);
    let (mine, _) = exploitability(&g, &sol.strategy(true, None));
    // The same strategy through the referee's tree + best response.
    let paths: Vec<u64> = rg.tree.nodes.iter().map(|nd| nd.path.iter().fold(0u64, |p, &a| step_path(p, a))).collect();
    let mut buf = [0f32; 24];
    let s = Strategy::from_fn(&rg.tree, |id, a, n, h| {
        if !sol.probs(paths[id], h, n, false, &mut buf) {
            buf[..n].iter_mut().for_each(|x| *x = 1.0 / n as f32);
        }
        buf[a] as f64
    });
    let theirs = rg.exploitability(&s);
    // Hands whose average is unreliable fall back to uniform here, so allow a small gap.
    assert!((mine - theirs).abs() < 0.02 * root.pot() as f64 + 1e-6, "mine {mine} referee {theirs}");
    // Reference CFR+ with the same number of iterations has the same exploitability.
    let eq = rg.solve(300);
    let eref = rg.exploitability(&eq);
    assert!((mine - eref).abs() < 0.02 * root.pot() as f64, "vector {mine} referee cfr+ {eref}");
    assert!(mine < 0.05 * root.pot() as f64, "river exploitability {mine} chips of pot {}", root.pot());
}

#[test]
fn turn_vector_exploitability_decreases_with_iterations() {
    let root = root_at(Street::Turn);
    let board = cs("2c7d9hJs");
    let r = ranges(&board, 9);
    let (sz, later) = (default_sizing(), later_sizing());
    let none = Extras::new();
    let frozen = HashMap::new();
    let g = VecGame { root, board: &board, seats: [1, 0], ranges: r, sizing: &sz, later: &later, extras: &none, me: None, my_hole: None, frozen: &frozen, pooled: None };
    let mut last = f64::INFINITY;
    for iters in [40u64, 400] {
        let vp = VecParams { iters, plus: true, keep_all: true, seed: 2, ..Default::default() };
        let sol = solve_vec(&g, &vp);
        let (e, _) = exploitability(&g, &sol.strategy(true, None));
        assert!(e < last, "iters {iters}: {e} vs {last}");
        last = e;
    }
    assert!(last < 0.5 * root.pot() as f64, "turn exploitability {last} vs pot {}", root.pot());
}
