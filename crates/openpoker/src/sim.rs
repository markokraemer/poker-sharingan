//! Replay of the table's actions onto an `nlhe::State`, plus the hero's bench `Player`.
//!
//! The simulation is *observational*: it never decides what the table does, it only applies
//! what the server reports (`player_action`) and cross-checks the engine against the
//! server's own numbers (pot, stacks, legal actions). Disagreements are collected as
//! `issues` (strings) and end up in the trace; the session treats a failed `apply_obs` as a
//! desync and falls back to a passive legal action.

use crate::layout::{scale, unscale, Layout};
use crate::proto::{ActionObs, ObsKind, ValidSet};
use bench::policy::{Dist, Event, Player, PolicyFactory, View};
use cards::Card;
use nlhe::{Action, State, Step, Street};

pub struct Sim {
    pub layout: Layout,
    pub state: State,
    pub hero_op: usize,
    pub hero_nl: usize,
    pub hole: [Card; 2],
    pub board: Vec<Card>,
    pub history: Vec<Event>,
    player: Option<Box<dyn Player>>,
    /// Non-fatal disagreements between engine and server since the last `take_issues`.
    issues: Vec<String>,
    /// Applied actions as (open poker seat, nlhe action in our chips), in order.
    pub applied: Vec<(usize, Action)>,
    /// Board cards visible when the last betting round closed with a showdown ahead
    /// (`Some(5)` = ordinary river showdown, `Some(<5)` = all-in run-out).
    pub lock_len: Option<usize>,
}

/// What `decide` produced.
#[derive(Clone, Debug)]
pub struct Decision {
    /// Full distribution over concrete nlhe actions (empty for non-exposing players).
    pub dist: Vec<(Action, f64)>,
    pub chosen: Action,
    /// Policy diagnostic for this decision (e.g. a search fallback reason).
    pub note: Option<String>,
}

impl Sim {
    /// `factory = None` builds a bare replay (contribution bookkeeping only).
    pub fn new(
        layout: Layout,
        hero_op: usize,
        hole: [Card; 2],
        factory: Option<&dyn PolicyFactory>,
        seed: u64,
    ) -> Result<Sim, String> {
        let hero_nl = layout
            .nl(hero_op)
            .ok_or_else(|| format!("hero seat {hero_op} not dealt in"))?;
        let state = State::new(&layout.cfg);
        let player = factory.map(|f| f.new_player(hero_nl, &layout.cfg, seed));
        let mut sim = Sim {
            layout,
            state,
            hero_op,
            hero_nl,
            hole,
            board: Vec::new(),
            history: Vec::new(),
            player,
            issues: Vec::new(),
            applied: Vec::new(),
            lock_len: None,
        };
        // Phantom seats (embedded short-handed game) fold first.
        for k in 0..sim.layout.prefold {
            let nl = 2 + k;
            if sim.state.to_act() != Some(nl) {
                return Err(format!("phantom fold: engine expects seat {:?}, not {nl}", sim.state.to_act()));
            }
            sim.push(nl, Action::Fold);
        }
        Ok(sim)
    }

    fn push(&mut self, nl: usize, a: Action) -> Step {
        let pre = self.state;
        let street = pre.street();
        let step = self.state.apply(a);
        if self.state.showdown() && self.lock_len.is_none() {
            self.lock_len = Some(street.board_len());
        }
        self.history.push(Event { seat: nl as u8, street, action: a });
        if let Some(p) = self.player.as_mut() {
            p.observe(&pre, nl, a);
        }
        step
    }

    pub fn take_issues(&mut self) -> Vec<String> {
        std::mem::take(&mut self.issues)
    }

    /// Apply a reported action. `Err` = the action cannot be placed (desync); the state is
    /// unchanged in that case.
    pub fn apply_obs(&mut self, o: &ActionObs) -> Result<(), String> {
        if o.kind == ObsKind::Ignored {
            return Ok(());
        }
        let nl = self
            .layout
            .nl(o.seat)
            .ok_or_else(|| format!("action from seat {} which is not dealt in", o.seat))?;
        if self.state.is_terminal() {
            return Err(format!("action from seat {} after the hand ended", o.seat));
        }
        if self.state.to_act() != Some(nl) {
            return Err(format!(
                "out of turn: engine expects nlhe seat {:?}, got seat {} (nlhe {nl})",
                self.state.to_act(),
                o.seat
            ));
        }
        let l = self.state.legal();
        let note = |s: String, issues: &mut Vec<String>| issues.push(format!("seat {}: {s}", o.seat));
        let mut issues = Vec::new();
        let act = match o.kind {
            ObsKind::Fold => {
                if l.can_fold {
                    Action::Fold
                } else {
                    note("fold while check is free; treated as check".into(), &mut issues);
                    Action::Check
                }
            }
            ObsKind::Check => {
                if l.can_check {
                    Action::Check
                } else {
                    note("check while facing a bet; treated as call".into(), &mut issues);
                    Action::Call
                }
            }
            ObsKind::Call => {
                if l.can_check {
                    Action::Check
                } else {
                    Action::Call
                }
            }
            ObsKind::Raise => {
                let Some(x) = o.amount else {
                    return Err(format!("raise from seat {} without amount", o.seat));
                };
                // `amount_mode: "to_total"` (the only mode seen for raises) is raise-to on the
                // street; if a server ever says "incremental", add it to what the actor has in.
                let t = if o.amount_mode.as_deref() == Some("incremental") {
                    self.state.street_bet(nl) + scale(x)
                } else {
                    scale(x)
                };
                if !l.can_raise {
                    note(format!("raise to {x} but engine cannot raise; treated as call"), &mut issues);
                    if l.can_check { Action::Check } else { Action::Call }
                } else if t < l.min_raise_to || t > l.max_raise_to {
                    let c = t.clamp(l.min_raise_to, l.max_raise_to);
                    note(
                        format!("raise to {x} outside engine range {}..{} (op chips); clamped", unscale(l.min_raise_to), unscale(l.max_raise_to)),
                        &mut issues,
                    );
                    Action::RaiseTo(c)
                } else {
                    Action::RaiseTo(t)
                }
            }
            ObsKind::AllIn => {
                if l.can_raise {
                    if let Some(x) = o.amount {
                        let t = scale(x);
                        // `amount` may be the raise-to total or the chips added; both must
                        // end at the stack, so the engine's all-in is authoritative.
                        if t != l.max_raise_to && t != 0 {
                            note(format!("all_in amount {x} != engine all-in {}", unscale(l.max_raise_to)), &mut issues);
                        }
                    }
                    Action::RaiseTo(l.max_raise_to)
                } else if l.can_check {
                    Action::Check
                } else {
                    Action::Call
                }
            }
            ObsKind::Ignored => unreachable!(),
        };
        self.push(nl, act);
        self.applied.push((o.seat, act));
        // Cross-checks against the server's own numbers. Not for `all_in`: the live server's
        // broadcast for that action carries the *pre-action* stack/pot (contribution_delta 0),
        // while the next event's `pot_before` already includes the chips.
        if o.kind == ObsKind::AllIn {
            self.issues.extend(issues);
            return Ok(());
        }
        if let Some(s) = o.stack {
            let mine = unscale(self.state.stack(nl));
            if mine != s {
                issues.push(format!("seat {}: stack after action server {s} vs engine {mine}", o.seat));
            }
        }
        if let Some(p) = o.pot {
            let mine = unscale(self.state.pot());
            if mine != p {
                issues.push(format!("pot after seat {}'s action server {p} vs engine {mine}", o.seat));
            }
        }
        self.issues.extend(issues);
        Ok(())
    }

    /// Reveal board cards (full board so far). Calls `observe_board` for each new street.
    pub fn set_board(&mut self, board: &[Card]) {
        if board.len() <= self.board.len() {
            return;
        }
        let mut lens: Vec<usize> = Vec::new();
        for to in [3usize, 4, 5] {
            if to > self.board.len() && to <= board.len() {
                lens.push(to);
            }
        }
        for to in lens {
            if let Some(p) = self.player.as_mut() {
                p.observe_board(&board[..to]);
            }
        }
        self.board = board.to_vec();
    }

    pub fn hero_to_act(&self) -> bool {
        !self.state.is_terminal() && self.state.to_act() == Some(self.hero_nl)
    }

    pub fn street(&self) -> Street {
        self.state.street()
    }

    pub fn pot_op(&self) -> i64 {
        unscale(self.state.pot())
    }

    pub fn contributed_op(&self, op_seat: usize) -> Option<i64> {
        self.layout.nl(op_seat).map(|nl| unscale(self.state.contributed(nl)))
    }

    pub fn street_bet_op(&self, op_seat: usize) -> Option<i64> {
        self.layout.nl(op_seat).map(|nl| unscale(self.state.street_bet(nl)))
    }

    pub fn stack_op(&self, op_seat: usize) -> Option<i64> {
        self.layout.nl(op_seat).map(|nl| unscale(self.state.stack(nl)))
    }

    /// Compare the engine's legal actions for the hero with the server's. Returns issues.
    pub fn compare_legal(&self, v: &ValidSet, server_pot: Option<i64>) -> Vec<String> {
        let mut out = Vec::new();
        if !self.hero_to_act() {
            out.push(format!(
                "engine says nlhe seat {:?} to act, hero is {}",
                self.state.to_act(),
                self.hero_nl
            ));
            return out;
        }
        let l = self.state.legal();
        if let Some(p) = server_pot {
            if p != self.pot_op() {
                out.push(format!("pot server {p} vs engine {}", self.pot_op()));
            }
        }
        if l.can_check != v.check {
            out.push(format!("check offered: server {} engine {}", v.check, l.can_check));
        }
        // The server also lists fold when checking is free; only a missing fold is a problem.
        if l.can_fold && !v.fold {
            out.push("engine expects fold to be offered, server does not".into());
        }
        if let Some(c) = v.call {
            let e = unscale(l.call_amount);
            // Servers may report the call as the total to match instead of the increment.
            if c != e {
                out.push(format!("call amount server {c} vs engine {e}"));
            }
        } else if !l.can_check {
            out.push("engine expects a call, server offers none".into());
        }
        match (v.raise, l.can_raise) {
            (Some((lo, hi)), true) => {
                if unscale(l.min_raise_to) != lo {
                    out.push(format!("min raise-to server {lo} vs engine {}", unscale(l.min_raise_to)));
                }
                if unscale(l.max_raise_to) != hi {
                    out.push(format!("max raise-to server {hi} vs engine {}", unscale(l.max_raise_to)));
                }
            }
            (Some(_), false) => out.push("server offers raise, engine cannot raise".into()),
            (None, true) => {
                if v.all_in.is_none() {
                    out.push("engine can raise, server offers neither raise nor all_in".into());
                }
            }
            (None, false) => {}
        }
        out
    }

    /// Ask the hero's policy. `u` in [0,1) is the sampling draw.
    pub fn decide(&mut self, u: f64) -> Result<Decision, String> {
        if !self.hero_to_act() {
            return Err("hero is not to act".into());
        }
        let need = self.state.street().board_len();
        if self.board.len() < need {
            return Err(format!("board has {} cards, street needs {need}", self.board.len()));
        }
        let hole = self.hole;
        let seat = self.hero_nl;
        let player = self.player.as_mut().ok_or("simulation has no policy")?;
        let view = View {
            cfg: &self.layout.cfg,
            state: &self.state,
            board: &self.board,
            history: &self.history,
            seat,
        };
        if player.exposes_probs() {
            let d: Dist = player.dist(&view, hole);
            if d.is_empty() {
                return Err("policy returned an empty distribution".into());
            }
            let i = d.sample(u);
            let dist = (0..d.len()).map(|k| (d.action(k), d.prob(k))).collect();
            let note = player.take_note();
            Ok(Decision { dist, chosen: d.action(i), note })
        } else {
            let a = player.act(&view, hole, u);
            let note = player.take_note();
            Ok(Decision { dist: Vec::new(), chosen: a, note })
        }
    }

    pub fn is_terminal(&self) -> bool {
        self.state.is_terminal()
    }
}
