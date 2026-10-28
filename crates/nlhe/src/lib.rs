//! Fast no-limit hold'em state machine for 2-6 players.
//!
//! `State` is a small `Copy` struct (fixed arrays, no heap); `apply` and `legal` are
//! allocation-free. The same engine drives MCCFR and real matches, so the rules are exact:
//! side pots, short all-ins, min-raise tracking.
//!
//! # Seats and blinds
//! Seat 0 = small blind, seat 1 = big blind, seats 2.. follow, the last seat is the button
//! (for 2 players the button is the SB, seat 0: heads-up convention, as used by Slumbot).
//! - Preflop first to act: seat 2 (left of BB); heads-up: seat 0 (SB/button).
//! - Postflop first to act: first non-folded, non-all-in seat left of the button, i.e. seat 0
//!   (SB) for 3+ players and seat 1 (BB) heads-up.
//! - Odd chips in a split pot go one each to the winners in order starting left of the button
//!   (seat 0, 1, 2, ... for 3+ players; seat 1 then 0 heads-up).
//!
//! # Betting rules
//! - `RaiseTo(t)`: `t` is the total committed on the current street after the action; it covers
//!   bets, raises and all-ins. Legal range is `[min_raise_to, max_raise_to]` where
//!   `min_raise_to = cur_bet + last_full_raise` (at least BB) and `max_raise_to` is all-in.
//!   If all-in is below the min raise it is still legal (a short all-in).
//! - A short all-in (increment < last full raise) does NOT reopen betting: players who already
//!   acted since the last full raise may only call or fold. Players yet to act may still raise
//!   (min raise unchanged, measured from the new bet). Cumulative short all-ins are not summed.
//! - Raising is disallowed when no opponent can still act (everyone else folded or all-in).
//! - Blinds are capped at the stack (a short blind is all-in).
//!
//! # Streets and chance
//! `apply` advances the street automatically when the round closes and returns a [`Step`]:
//! `NewStreet(s)` means board cards for `s` must be revealed (a chance event) and decisions
//! remain; `Terminal` means no decisions remain. If fewer than two players can still act when
//! a round closes, the board runs out: `apply` returns `Terminal`, `showdown()` is true and
//! `street()` is `River`. `payoffs` takes the whole 5-card board, so the caller samples chance.

pub mod abstraction;
pub mod tree;

use cards::{bit, evaluate, Card, Hand};

pub use abstraction::{ActionList, BetSizing, History};

pub const MAX_PLAYERS: usize = 6;
const NONE: u8 = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Street {
    Preflop = 0,
    Flop = 1,
    Turn = 2,
    River = 3,
}

impl Street {
    pub const ALL: [Street; 4] = [Street::Preflop, Street::Flop, Street::Turn, Street::River];
    pub fn idx(self) -> usize {
        self as usize
    }
    /// Number of board cards visible on this street.
    pub fn board_len(self) -> usize {
        [0, 3, 4, 5][self as usize]
    }
    fn from_u8(v: u8) -> Street {
        Self::ALL[v as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameConfig {
    pub players: usize,
    /// Starting stack for every seat unless `stacks` is set.
    pub stack: i32,
    pub small_blind: i32,
    pub big_blind: i32,
    /// Optional per-seat starting stacks (seats `0..players`), e.g. for external sites.
    pub stacks: Option<[i32; MAX_PLAYERS]>,
}

impl GameConfig {
    /// Paper setup: 10,000 stacks, 50/100 blinds.
    pub fn new(players: usize) -> Self {
        GameConfig {
            players,
            stack: 10_000,
            small_blind: 50,
            big_blind: 100,
            stacks: None,
        }
    }
    pub fn with_stacks(mut self, s: &[i32]) -> Self {
        let mut a = [0; MAX_PLAYERS];
        a[..s.len()].copy_from_slice(s);
        self.stacks = Some(a);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Fold,
    Check,
    Call,
    /// Total committed on this street after the action (bet, raise or all-in).
    RaiseTo(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Legal {
    pub can_fold: bool,
    pub can_check: bool,
    /// Chips to call (capped at the stack); 0 when checking.
    pub call_amount: i32,
    pub can_raise: bool,
    /// Valid only when `can_raise`. May equal `max_raise_to` (short all-in).
    pub min_raise_to: i32,
    pub max_raise_to: i32,
}

/// Result of `apply`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Same street, next player to act.
    Continue,
    /// Round closed; chance reveals the board for this street, then `to_act()` decides.
    NewStreet(Street),
    /// No decisions remain (fold-out or showdown, possibly after a board run-out).
    Terminal,
}

/// A full deal: hole cards per seat and the 5 board cards (flop, turn, river).
#[derive(Clone, Copy, Debug)]
pub struct Deal {
    pub holes: [[Card; 2]; MAX_PLAYERS],
    pub board: [Card; 5],
}

impl Deal {
    pub fn random(rng: &mut cards::Rng) -> Deal {
        let d = rng.deck();
        let mut holes = [[0; 2]; MAX_PLAYERS];
        for (i, h) in holes.iter_mut().enumerate() {
            *h = [d[2 * i], d[2 * i + 1]];
        }
        Deal {
            holes,
            board: [d[12], d[13], d[14], d[15], d[16]],
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct State {
    stack: [i32; MAX_PLAYERS],
    bet: [i32; MAX_PLAYERS],
    total: [i32; MAX_PLAYERS],
    cur_bet: i32,
    last_raise: i32,
    big_blind: i32,
    n: u8,
    street: u8,
    to_act: u8,
    raises: u8,
    folded: u8,
    allin: u8,
    pending: u8,
    no_raise: u8,
    /// 0 = live, 1 = everyone folded to one player, 2 = showdown.
    term: u8,
}

impl State {
    pub fn new(cfg: &GameConfig) -> State {
        let n = cfg.players;
        assert!((2..=MAX_PLAYERS).contains(&n), "2-6 players");
        let mut s = State {
            stack: cfg.stacks.unwrap_or([cfg.stack; MAX_PLAYERS]),
            bet: [0; MAX_PLAYERS],
            total: [0; MAX_PLAYERS],
            cur_bet: 0,
            last_raise: cfg.big_blind,
            big_blind: cfg.big_blind,
            n: n as u8,
            street: 0,
            to_act: NONE,
            raises: 0,
            folded: 0,
            allin: 0,
            pending: 0,
            no_raise: 0,
            term: 0,
        };
        for i in n..MAX_PLAYERS {
            s.stack[i] = 0;
        }
        s.pay(0, cfg.small_blind.min(s.stack[0]));
        s.pay(1, cfg.big_blind.min(s.stack[1]));
        s.cur_bet = s.bet[0].max(s.bet[1]);
        if !s.open_round(if n == 2 { 0 } else { 2 }) {
            s.term = 2;
            s.street = Street::River as u8;
        }
        s
    }

    // ---- accessors ----
    #[inline]
    pub fn num_players(&self) -> usize {
        self.n as usize
    }
    #[inline]
    pub fn street(&self) -> Street {
        Street::from_u8(self.street)
    }
    #[inline]
    pub fn to_act(&self) -> Option<usize> {
        if self.to_act == NONE {
            None
        } else {
            Some(self.to_act as usize)
        }
    }
    #[inline]
    pub fn is_terminal(&self) -> bool {
        self.term != 0
    }
    /// Terminal with two or more players left (board fully dealt, cards compared).
    #[inline]
    pub fn showdown(&self) -> bool {
        self.term == 2
    }
    #[inline]
    pub fn pot(&self) -> i32 {
        self.total.iter().sum()
    }
    /// Remaining chips behind (not yet committed).
    #[inline]
    pub fn stack(&self, seat: usize) -> i32 {
        self.stack[seat]
    }
    /// Committed on the current street.
    #[inline]
    pub fn street_bet(&self, seat: usize) -> i32 {
        self.bet[seat]
    }
    /// Committed over the whole hand.
    #[inline]
    pub fn contributed(&self, seat: usize) -> i32 {
        self.total[seat]
    }
    #[inline]
    pub fn is_folded(&self, seat: usize) -> bool {
        self.folded >> seat & 1 != 0
    }
    #[inline]
    pub fn is_all_in(&self, seat: usize) -> bool {
        self.allin >> seat & 1 != 0
    }
    /// Highest street commitment (the amount to match).
    #[inline]
    pub fn cur_bet(&self) -> i32 {
        self.cur_bet
    }
    /// Size of the last full raise increment this street (at least BB).
    #[inline]
    pub fn last_raise(&self) -> i32 {
        self.last_raise
    }
    /// Raise/bet actions taken this street (preflop blinds do not count).
    #[inline]
    pub fn raises_this_street(&self) -> u32 {
        self.raises as u32
    }
    #[inline]
    pub fn live_count(&self) -> u32 {
        self.live_mask().count_ones()
    }
    #[inline]
    pub fn active_count(&self) -> u32 {
        self.active_mask().count_ones()
    }

    #[inline]
    fn all_mask(&self) -> u8 {
        (1u8 << self.n) - 1
    }
    #[inline]
    fn live_mask(&self) -> u8 {
        self.all_mask() & !self.folded
    }
    #[inline]
    fn active_mask(&self) -> u8 {
        self.live_mask() & !self.allin
    }

    #[inline]
    fn pay(&mut self, p: usize, x: i32) {
        self.stack[p] -= x;
        self.bet[p] += x;
        self.total[p] += x;
        if self.stack[p] == 0 {
            self.allin |= 1 << p;
        }
    }

    /// First pending seat scanning cyclically from `start`.
    #[inline]
    fn first_pending(&self, start: usize) -> u8 {
        let n = self.n as usize;
        let rot = (self.pending as u16) | ((self.pending as u16) << n);
        let m = rot >> start;
        ((m.trailing_zeros() as usize + start) % n) as u8
    }

    /// Set up betting for the (already reset) current street. False if no decision is needed.
    fn open_round(&mut self, start: usize) -> bool {
        let active = self.active_mask();
        let mut pending = active;
        if active.count_ones() <= 1 {
            // A lone active player only acts when facing a bet from an all-in opponent.
            if active == 0 || self.bet[active.trailing_zeros() as usize] >= self.cur_bet {
                pending = 0;
            }
        }
        self.pending = pending;
        if pending == 0 {
            self.to_act = NONE;
            return false;
        }
        self.to_act = self.first_pending(start);
        true
    }

    fn close_round(&mut self) -> Step {
        if self.street == Street::River as u8 {
            self.term = 2;
            self.to_act = NONE;
            return Step::Terminal;
        }
        self.street += 1;
        self.bet = [0; MAX_PLAYERS];
        self.cur_bet = 0;
        self.last_raise = self.big_blind;
        self.raises = 0;
        self.no_raise = 0;
        let start = if self.n == 2 { 1 } else { 0 };
        if self.open_round(start) {
            Step::NewStreet(self.street())
        } else {
            self.street = Street::River as u8;
            self.term = 2;
            Step::Terminal
        }
    }

    /// Legal actions for the player to act. Must not be terminal.
    #[inline]
    pub fn legal(&self) -> Legal {
        let p = self.to_act as usize;
        debug_assert!(p < MAX_PLAYERS);
        let owe = self.cur_bet - self.bet[p];
        let max_to = self.bet[p] + self.stack[p];
        let others_active = self.active_mask() & !(1 << p) != 0;
        let can_raise = others_active && self.no_raise >> p & 1 == 0 && max_to > self.cur_bet;
        Legal {
            can_fold: owe > 0,
            can_check: owe == 0,
            call_amount: owe.min(self.stack[p]),
            can_raise,
            min_raise_to: (self.cur_bet + self.last_raise).min(max_to),
            max_raise_to: max_to,
        }
    }

    /// Apply an action for `to_act()`. Panics (debug) on illegal actions.
    #[inline]
    pub fn apply(&mut self, a: Action) -> Step {
        let p = self.to_act as usize;
        debug_assert!(p < MAX_PLAYERS, "apply on terminal state");
        let bit = 1u8 << p;
        match a {
            Action::Fold => {
                debug_assert!(self.cur_bet > self.bet[p], "fold when check is free");
                self.folded |= bit;
                self.pending &= !bit;
            }
            Action::Check | Action::Call => {
                debug_assert!(a != Action::Check || self.cur_bet == self.bet[p]);
                let c = (self.cur_bet - self.bet[p]).min(self.stack[p]);
                self.pay(p, c);
                self.pending &= !bit;
            }
            Action::RaiseTo(t) => {
                debug_assert!({
                    let l = self.legal();
                    l.can_raise && t >= l.min_raise_to && t <= l.max_raise_to
                });
                self.pay(p, t - self.bet[p]);
                let inc = t - self.cur_bet;
                let others = self.active_mask() & !bit;
                if inc >= self.last_raise {
                    self.last_raise = inc;
                    self.pending = others;
                    self.no_raise = 0;
                } else {
                    // Short all-in: those who already acted may only call/fold.
                    self.no_raise |= others & !self.pending;
                    self.pending = (self.pending | others) & !bit;
                }
                self.cur_bet = t;
                self.raises += 1;
            }
        }
        if self.live_mask().count_ones() == 1 {
            self.term = 1;
            self.to_act = NONE;
            return Step::Terminal;
        }
        self.pending &= self.active_mask();
        if self.pending == 0 {
            return self.close_round();
        }
        self.to_act = self.first_pending(p + 1);
        Step::Continue
    }

    /// Net chip delta per seat (sum == 0). Requires a terminal state; seats `>= players` are 0.
    pub fn payoffs(&self, deal: &Deal) -> [i32; MAX_PLAYERS] {
        debug_assert!(self.is_terminal());
        let n = self.n as usize;
        let live = self.live_mask();
        let mut out = [0i32; MAX_PLAYERS];
        for i in 0..n {
            out[i] = -self.total[i];
        }
        if live.count_ones() == 1 {
            out[live.trailing_zeros() as usize] += self.pot();
            return out;
        }
        let board: Hand = deal.board.iter().fold(0, |m, &c| m | bit(c));
        let mut rank = [0u32; MAX_PLAYERS];
        for i in 0..n {
            if live >> i & 1 != 0 {
                rank[i] = evaluate(bit(deal.holes[i][0]) | bit(deal.holes[i][1]) | board);
            }
        }
        // Left of the button: seat 1 heads-up, else seat 0.
        let first = if n == 2 { 1 } else { 0 };
        let mut prev = 0;
        loop {
            let level = (0..n)
                .filter(|&i| live >> i & 1 != 0 && self.total[i] > prev)
                .map(|i| self.total[i])
                .min();
            let Some(level) = level else { break };
            let pot: i32 = (0..n)
                .map(|i| self.total[i].min(level) - self.total[i].min(prev))
                .sum();
            let eligible = |i: usize| live >> i & 1 != 0 && self.total[i] >= level;
            let best = (0..n)
                .filter(|&i| eligible(i))
                .map(|i| rank[i])
                .max()
                .unwrap();
            let mut winners = [0usize; MAX_PLAYERS];
            let mut w = 0;
            for k in 0..n {
                let i = (first + k) % n;
                if eligible(i) && rank[i] == best {
                    winners[w] = i;
                    w += 1;
                }
            }
            let (share, odd) = (pot / w as i32, (pot % w as i32) as usize);
            for (k, &i) in winners[..w].iter().enumerate() {
                out[i] += share + (k < odd) as i32;
            }
            prev = level;
        }
        debug_assert!(
            (0..n).all(|i| self.total[i] <= prev),
            "folded chips above top live level"
        );
        out
    }
}
