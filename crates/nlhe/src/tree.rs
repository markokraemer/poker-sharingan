//! Betting-tree size counting under an action abstraction (players' actions only, no chance).

use crate::abstraction::{abstract_actions, ActionList, BetSizing};
use crate::{GameConfig, State, Step, Street};
use cards::Rng;
use std::time::Instant;

#[derive(Clone, Copy, Debug, Default)]
pub struct TreeCount {
    /// Nodes where a player must act (= action sequences, incl. the root).
    pub decisions: u64,
    /// Nodes where play ended, or where the street limit was reached.
    pub leaves: u64,
    /// False if the deadline hit before the DFS finished.
    pub complete: bool,
}

struct Dfs<'a> {
    sizing: &'a BetSizing,
    last_street: Street,
    deadline: Option<Instant>,
    out: TreeCount,
    timed_out: bool,
}

impl Dfs<'_> {
    fn go(&mut self, s: &State) {
        self.out.decisions += 1;
        if self.out.decisions & 0xFFFFF == 0 {
            if let Some(d) = self.deadline {
                self.timed_out |= Instant::now() > d;
            }
        }
        let mut list = ActionList::new();
        abstract_actions(s, self.sizing, &mut list);
        for &a in list.as_slice() {
            if self.timed_out {
                return;
            }
            let mut t = *s;
            match t.apply(a) {
                Step::Continue => self.go(&t),
                Step::NewStreet(st) if st <= self.last_street => self.go(&t),
                _ => self.out.leaves += 1,
            }
        }
    }
}

/// Exact DFS count through `last_street` (inclusive; reaching the next street is a leaf).
pub fn count_tree(
    cfg: &GameConfig,
    sizing: &BetSizing,
    last_street: Street,
    deadline: Option<Instant>,
) -> TreeCount {
    let mut d = Dfs {
        sizing,
        last_street,
        deadline,
        out: TreeCount::default(),
        timed_out: false,
    };
    d.go(&State::new(cfg));
    d.out.complete = !d.timed_out;
    d.out
}

/// Knuth's random-path estimator of the number of decision nodes (unbiased, high variance).
pub fn estimate_tree(
    cfg: &GameConfig,
    sizing: &BetSizing,
    last_street: Street,
    samples: u64,
    rng: &mut Rng,
) -> f64 {
    let mut sum = 0.0;
    for _ in 0..samples {
        let mut s = State::new(cfg);
        let (mut weight, mut est) = (1.0f64, 0.0f64);
        let mut list = ActionList::new();
        loop {
            est += weight;
            abstract_actions(&s, sizing, &mut list);
            weight *= list.len() as f64;
            match s.apply(list.get(rng.below(list.len() as u32) as usize)) {
                Step::Continue => {}
                Step::NewStreet(st) if st <= last_street => {}
                _ => break,
            }
        }
        sum += est;
    }
    sum / samples as f64
}
