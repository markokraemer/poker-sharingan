//! Checked replay of an external action history on the `nlhe` engine, plus the glue that feeds
//! a bench [`Player`] and asks it for a distribution.
//!
//! External sources (PokerBench) give sizes in big blinds, rounded, and omit folds of players
//! who simply never acted. [`Replay`] therefore (a) never panics on illegal input (every
//! action is validated and returns `Err`), (b) can auto-fold skipped seats, and (c) snaps
//! rounded raise sizes to the legal range when they are within 1bb of a bound.

use bench::policy::{Dist, Event, Player, PolicyFactory, View};
use cards::Card;
use nlhe::abstraction::raise_fraction;
use nlhe::{Action, GameConfig, State, Step as NStep, Street};

/// Chips per big blind in the paper setup (50/100 blinds, 10,000 stacks = 100bb).
pub const BB: i32 = 100;

/// One replayed engine step, enough to re-feed any [`Player`].
#[derive(Clone, Copy, Debug)]
pub enum Step {
    /// Action by `seat` taken at `pre` (state before the action); `blen` board cards visible.
    Act {
        pre: State,
        seat: usize,
        action: Action,
        blen: usize,
    },
    /// Board revealed up to this many cards.
    Board(usize),
}

pub struct Replay {
    pub cfg: GameConfig,
    pub state: State,
    /// Full board known to the caller (the replay reveals a prefix as streets advance).
    pub board: Vec<Card>,
    pub blen: usize,
    pub steps: Vec<Step>,
    pub events: Vec<Event>,
}

pub fn err<T>(s: &str) -> Result<T, String> {
    Err(s.to_string())
}

impl Replay {
    /// `players`-max, 100bb stacks, 0.5/1bb blinds. `board` is the (eventual) board.
    pub fn new(players: usize, board: &[Card]) -> Replay {
        let cfg = GameConfig::new(players);
        Replay {
            cfg,
            state: State::new(&cfg),
            board: board.to_vec(),
            blen: 0,
            steps: Vec::new(),
            events: Vec::new(),
        }
    }

    pub fn visible_board(&self) -> &[Card] {
        &self.board[..self.blen]
    }

    pub fn to_act(&self) -> Option<usize> {
        self.state.to_act()
    }

    /// Validate and apply `a`. Returns the engine step.
    pub fn apply(&mut self, a: Action) -> Result<NStep, String> {
        let Some(seat) = self.state.to_act() else {
            return err("terminal");
        };
        let l = self.state.legal();
        let ok = match a {
            Action::Fold => l.can_fold,
            Action::Check => l.can_check,
            Action::Call => !l.can_check,
            Action::RaiseTo(t) => l.can_raise && t >= l.min_raise_to && t <= l.max_raise_to,
        };
        if !ok {
            return err("illegal action");
        }
        let pre = self.state;
        let st = self.state.street();
        let step = self.state.apply(a);
        self.steps.push(Step::Act {
            pre,
            seat,
            action: a,
            blen: self.blen,
        });
        self.events.push(Event {
            seat: seat as u8,
            street: st,
            action: a,
        });
        if let NStep::NewStreet(s) = step {
            let n = s.board_len();
            if self.board.len() < n {
                return err("board too short");
            }
            self.blen = n;
            self.steps.push(Step::Board(n));
        }
        Ok(step)
    }

    /// Fold every seat before `seat` that is to act (they were skipped in the source line).
    pub fn skip_to(&mut self, seat: usize) -> Result<(), String> {
        while let Some(t) = self.state.to_act() {
            if t == seat {
                return Ok(());
            }
            if !self.state.legal().can_fold {
                return err("skipped seat cannot fold");
            }
            if let NStep::Terminal = self.apply(Action::Fold)? {
                return err("hand ended while skipping");
            }
            if self.state.street() != self.steps_street() {
                return err("street advanced while skipping");
            }
        }
        err("seat never reached")
    }

    fn steps_street(&self) -> Street {
        self.events
            .last()
            .map(|e| e.street)
            .unwrap_or(Street::Preflop)
    }

    /// Convert a raise-to size in chips (rounded source data) into a legal `RaiseTo`:
    /// within 1bb of the all-in or minimum bounds it snaps to them, otherwise it must be legal.
    pub fn snap_raise(&self, to: i32) -> Result<Action, String> {
        let l = self.state.legal();
        if !l.can_raise {
            return err("cannot raise");
        }
        let t = if to >= l.max_raise_to - BB {
            l.max_raise_to
        } else if to < l.min_raise_to {
            if l.min_raise_to - to <= BB {
                l.min_raise_to
            } else {
                return err("raise below minimum");
            }
        } else {
            to
        };
        Ok(Action::RaiseTo(t))
    }

    /// Fresh player for `seat`, fed with every step so far.
    pub fn player(&self, f: &dyn PolicyFactory, seat: usize, seed: u64) -> Box<dyn Player> {
        let mut p = f.new_player(seat, &self.cfg, seed);
        feed(&mut *p, &self.steps, &self.board);
        p
    }

    /// Distribution of `p` for `hole` at the current decision.
    pub fn dist(&self, p: &mut dyn Player, hole: [Card; 2]) -> Dist {
        let seat = self.state.to_act().expect("decision");
        let v = View {
            cfg: &self.cfg,
            state: &self.state,
            board: self.visible_board(),
            history: &self.events,
            seat,
        };
        p.dist(&v, hole)
    }
}

/// Feed recorded steps to a player (observe / observe_board in engine order).
pub fn feed(p: &mut dyn Player, steps: &[Step], board: &[Card]) {
    for s in steps {
        match s {
            Step::Act {
                pre, seat, action, ..
            } => p.observe(pre, *seat, *action),
            Step::Board(n) => p.observe_board(&board[..*n]),
        }
    }
}

/// Map a concrete policy action onto the index of the nearest option in `targets`
/// (same class; raises by log-ratio of the raise increment). `None` if no option has the class.
pub fn nearest_action(state: &State, targets: &[Action], a: Action) -> Option<usize> {
    match a {
        Action::Fold => targets.iter().position(|t| matches!(t, Action::Fold)),
        Action::Check | Action::Call => targets
            .iter()
            .position(|t| matches!(t, Action::Check | Action::Call)),
        Action::RaiseTo(to) => {
            let inc = |x: i32| ((x - state.cur_bet()).max(1)) as f64;
            let mut best: Option<(usize, f64)> = None;
            for (i, t) in targets.iter().enumerate() {
                if let Action::RaiseTo(tt) = t {
                    let d = (inc(to) / inc(*tt)).ln().abs();
                    if best.is_none_or(|(_, bd)| d < bd) {
                        best = Some((i, d));
                    }
                }
            }
            best.map(|b| b.0)
        }
    }
}

/// Pot fraction of a raise (for reports).
pub fn frac(state: &State, to: i32) -> f64 {
    raise_fraction(state, to)
}
