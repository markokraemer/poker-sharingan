//! History -> engine state.
//!
//! Two engines run in lockstep:
//!
//! * the **real** engine (`players` = dealt-in count, request chips, request blinds). It is the
//!   authority for legality: every history action is validated against it and the answer is
//!   clamped to it. Errors are reported in the caller's chips.
//! * the **scaled** engine, what the bot sees: chips x `100/big_blind` (the blueprint was trained
//!   at 50/100 blinds) and, for 3-5 players, embedded in the 6-max game by `6 - n` phantom
//!   players (seats `2..`) who fold immediately, so the blueprint sees a history that "folded
//!   around" (same trick as `arena/openpoker/src/layout.rs`). Heads-up is played natively on a
//!   2-player engine.
//!
//! When `big_blind` divides 100 the scaling is exact and both engines are isomorphic. Otherwise
//! raises are rounded/clamped into the scaled engine's legal range; if the engines ever disagree
//! (extremely rare) the hand is flagged `desync` and the decision falls back to a passive legal
//! action instead of a wrong one.

use crate::api::*;
use crate::error::ApiError;
use bench::policy::{Event, View};
use cards::{card_str, parse_card, Card};
use nlhe::{Action, GameConfig, Legal, State, Step, Street, MAX_PLAYERS};
use search::policy::{SearchPlayer, SearchPolicy};
use serde_json::json;

pub const MAX_SEAT: i64 = 9;
/// Largest chip amounts accepted (keeps every engine sum far from i32 overflow).
const MAX_BB: i64 = 1_000_000;
const MAX_STACK_BB: i64 = 100_000;
const MAX_STACK: i64 = 100_000_000;

pub fn street_name(s: Street) -> &'static str {
    ["preflop", "flop", "turn", "river"][s.idx()]
}

fn parse_street(s: &str) -> Option<Street> {
    match s.to_ascii_lowercase().as_str() {
        "preflop" => Some(Street::Preflop),
        "flop" => Some(Street::Flop),
        "turn" => Some(Street::Turn),
        "river" => Some(Street::River),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    PostSb,
    PostBb,
    Fold,
    Check,
    Call,
    Bet,
    Raise,
    AllIn,
}

fn parse_kind(s: &str) -> Option<Kind> {
    Some(match s.to_ascii_lowercase().replace('-', "_").as_str() {
        "post_sb" => Kind::PostSb,
        "post_bb" => Kind::PostBb,
        "fold" => Kind::Fold,
        "check" => Kind::Check,
        "call" => Kind::Call,
        "bet" => Kind::Bet,
        "raise" => Kind::Raise,
        "all_in" | "allin" => Kind::AllIn,
        _ => return None,
    })
}

pub fn parse_cards_field(what: &str, v: &[String]) -> Result<Vec<Card>, ApiError> {
    v.iter()
        .map(|s| parse_card(s).ok_or_else(|| ApiError::invalid("invalid_card", format!("{what}: {s:?} is not a card (expected e.g. \"Ah\", \"Td\")"))))
        .collect()
}

pub struct Hand {
    /// Table seat of each real-engine index, in engine order `[SB, BB, ..., last]`.
    pub order: Vec<usize>,
    pub real: State,
    pub real_cfg: GameConfig,
    pub scaled: State,
    pub scaled_cfg: GameConfig,
    /// Phantom players embedded before the first real UTG.
    pub prefold: usize,
    pub scale: f64,
    pub hero_ri: usize,
    pub hero_si: usize,
    pub hole: [Card; 2],
    pub board: Vec<Card>,
    /// Scaled-engine events (for the bot's `View`).
    pub history: Vec<Event>,
    pub player: Option<SearchPlayer>,
    /// Set when the scaled engine stopped matching the real one.
    pub desync: Option<String>,
    pub notes: Vec<String>,
    pub big_blind: i64,
}

impl Hand {
    fn sc(&self, x: i64) -> i32 {
        ((x as f64 * self.scale).round() as i64).clamp(0, 1 << 28) as i32
    }

    /// Validate `req` and replay its history. `policy = None` builds a bare (validation-only) replay.
    pub fn replay(req: &ActRequest, policy: Option<&SearchPolicy>, seed: u64) -> Result<Hand, ApiError> {
        let (sb, bb) = (req.small_blind, req.big_blind);
        if sb <= 0 || bb <= 0 || sb > bb || bb > MAX_BB {
            return Err(ApiError::invalid("bad_blinds", format!("need 0 < small_blind <= big_blind <= {MAX_BB}, got {sb}/{bb}")));
        }
        if !(0..=MAX_SEAT).contains(&req.button_seat) {
            return Err(ApiError::invalid("bad_seat", format!("button_seat {} outside 0..={MAX_SEAT}", req.button_seat)));
        }
        // ---- players
        let mut dealt: Vec<(usize, i64)> = vec![];
        let mut seen = [false; (MAX_SEAT + 1) as usize];
        for p in &req.players {
            if !(0..=MAX_SEAT).contains(&p.seat) {
                return Err(ApiError::invalid("bad_seat", format!("player seat {} outside 0..={MAX_SEAT}", p.seat)));
            }
            if std::mem::replace(&mut seen[p.seat as usize], true) {
                return Err(ApiError::invalid("duplicate_seat", format!("seat {} listed twice in players", p.seat)));
            }
            if !p.in_hand {
                continue;
            }
            if p.stack_start <= 0 || p.stack_start > MAX_STACK || p.stack_start > MAX_STACK_BB * bb {
                return Err(ApiError::invalid(
                    "bad_stack",
                    format!("seat {}: stack_start {} must be in 1..={} (and at most {MAX_STACK_BB} big blinds)", p.seat, p.stack_start, MAX_STACK),
                ));
            }
            dealt.push((p.seat as usize, p.stack_start));
        }
        dealt.sort_unstable();
        let n = dealt.len();
        if !(2..=MAX_PLAYERS).contains(&n) {
            return Err(ApiError::invalid("bad_player_count", format!("{n} players dealt in; the bot plays 2 to 6")));
        }
        if !(0..=MAX_SEAT).contains(&req.hero_seat) || !dealt.iter().any(|d| d.0 as i64 == req.hero_seat) {
            return Err(ApiError::invalid("hero_not_in_hand", format!("hero_seat {} is not among the dealt-in players", req.hero_seat)));
        }
        let hero_seat = req.hero_seat as usize;
        // ---- cards
        if req.hole_cards.len() != 2 {
            return Err(ApiError::invalid("invalid_card", format!("hole_cards must have exactly 2 cards, got {}", req.hole_cards.len())));
        }
        let hc = parse_cards_field("hole_cards", &req.hole_cards)?;
        let hole = [hc[0], hc[1]];
        let board = parse_cards_field("board", &req.board)?;
        if !matches!(board.len(), 0 | 3 | 4 | 5) {
            return Err(ApiError::invalid("board_mismatch", format!("board must have 0, 3, 4 or 5 cards, got {}", board.len())));
        }
        let mut used = 0u64;
        for &c in hole.iter().chain(board.iter()) {
            if used >> c & 1 == 1 {
                return Err(ApiError::invalid("duplicate_card", format!("card {} appears twice in hole_cards + board", card_str(c))));
            }
            used |= 1 << c;
        }
        // ---- seat order [SB, BB, ..., last]; heads-up the button is the SB (TABLE_API)
        let seats: Vec<usize> = dealt.iter().map(|d| d.0).collect();
        let b = req.button_seat as usize;
        let start = seats.iter().position(|&s| s > b).unwrap_or(0);
        let mut order: Vec<usize> = (0..n).map(|i| seats[(start + i) % n]).collect();
        if n == 2 && seats.contains(&b) {
            order = vec![b, *seats.iter().find(|&&s| s != b).unwrap()];
        }
        let stack_of = |seat: usize| dealt.iter().find(|d| d.0 == seat).map(|d| d.1).unwrap_or(0);
        let hero_ri = order.iter().position(|&s| s == hero_seat).unwrap();
        // ---- engines
        let scale = 100.0 / bb as f64;
        let prefold = if (3..=5).contains(&n) { MAX_PLAYERS - n } else { 0 };
        let si_of = |ri: usize| if ri < 2 { ri } else { ri + prefold };
        let mut rs = [0i32; MAX_PLAYERS];
        for (ri, &seat) in order.iter().enumerate() {
            rs[ri] = stack_of(seat) as i32;
        }
        let real_cfg = GameConfig { players: n, stack: 0, small_blind: sb as i32, big_blind: bb as i32, stacks: Some(rs) };
        let mut h = Hand {
            order,
            real: State::new(&real_cfg),
            real_cfg,
            scaled: State::new(&GameConfig::new(2)), // replaced below
            scaled_cfg: GameConfig::new(2),
            prefold,
            scale,
            hero_ri,
            hero_si: si_of(hero_ri),
            hole,
            board,
            history: vec![],
            player: None,
            desync: None,
            notes: vec![],
            big_blind: bb,
        };
        let mut ss = [0i32; MAX_PLAYERS];
        for (ri, &seat) in h.order.clone().iter().enumerate() {
            ss[si_of(ri)] = h.sc(stack_of(seat)).max(1);
        }
        for k in 0..prefold {
            ss[2 + k] = 100 * 100; // phantom: folds at once, size irrelevant
        }
        h.scaled_cfg = GameConfig {
            players: if prefold > 0 { MAX_PLAYERS } else { n },
            stack: 0,
            small_blind: h.sc(sb).max(1),
            big_blind: h.sc(bb).max(1),
            stacks: Some(ss),
        };
        h.scaled = State::new(&h.scaled_cfg);
        h.player = policy.map(|p| p.player(h.hero_si, &h.scaled_cfg, seed));
        for k in 0..prefold {
            let nl = 2 + k;
            if h.scaled.to_act() != Some(nl) {
                h.desync = Some(format!("phantom fold: scaled engine expects seat {:?}, not {nl}", h.scaled.to_act()));
                break;
            }
            h.push_scaled(nl, Action::Fold);
        }

        // ---- leading blind posts (optional)
        let posts = [(Kind::PostSb, h.order[0], sb), (Kind::PostBb, h.order[1], bb)];
        let mut idx = 0;
        while idx < req.history.len() {
            let ev = &req.history[idx];
            let Some(k @ (Kind::PostSb | Kind::PostBb)) = parse_kind(&ev.kind) else { break };
            let Some(&(ek, eseat, eblind)) = posts.get(idx) else {
                return Err(ApiError::invalid("bad_post", format!("history[{idx}]: too many blind posts")).with(json!({"index": idx})));
            };
            if k != ek || ev.seat != eseat as i64 {
                return Err(ApiError::invalid(
                    "bad_post",
                    format!("history[{idx}]: expected {} by seat {eseat}, got {} by seat {}", if ek == Kind::PostSb { "post_sb" } else { "post_bb" }, ev.kind, ev.seat),
                )
                .with(json!({"index": idx, "expected_seat": eseat})));
            }
            let want = eblind.min(stack_of(eseat));
            if let Some(to) = ev.to {
                if to != want {
                    return Err(ApiError::invalid("bad_amount", format!("history[{idx}]: blind post of {to}, expected {want}")).with(json!({"index": idx, "expected_to": want})));
                }
            }
            idx += 1;
        }

        // ---- actions
        for i in idx..req.history.len() {
            h.apply_event(i, &req.history[i])?;
        }

        // ---- the hero must be on the clock, with the right board
        if h.real.is_terminal() {
            return Err(ApiError::new(409, "hand_over", "the history ends the hand; there is no decision to make"));
        }
        let ri = h.real.to_act().unwrap();
        if ri != h.hero_ri {
            return Err(ApiError::invalid("not_hero_turn", format!("seat {} is to act, not hero seat {hero_seat}", h.order[ri]))
                .with(json!({"to_act_seat": h.order[ri], "hero_seat": hero_seat})));
        }
        let st = h.real.street();
        if h.board.len() != st.board_len() {
            return Err(ApiError::invalid(
                "board_mismatch",
                format!("the hand is on the {} which needs {} board cards, got {}", street_name(st), st.board_len(), h.board.len()),
            ));
        }
        if h.desync.is_none() && h.scaled.to_act() != Some(h.hero_si) {
            h.desync = Some("scaled engine does not have the hero to act".into());
        }
        if h.desync.is_some() {
            h.player = None;
        }
        Ok(h)
    }

    fn push_scaled(&mut self, nl: usize, a: Action) -> Step {
        let pre = self.scaled;
        let step = self.scaled.apply(a);
        self.history.push(Event { seat: nl as u8, street: pre.street(), action: a });
        if let Some(p) = self.player.as_mut() {
            use bench::policy::Player;
            p.observe(&pre, nl, a);
        }
        step
    }

    fn si(&self, ri: usize) -> usize {
        if ri < 2 {
            ri
        } else {
            ri + self.prefold
        }
    }

    /// Real action -> the action in the scaled game.
    fn to_scaled(&self, ra: Action, real_legal: &Legal) -> Action {
        let l = self.scaled.legal();
        match ra {
            Action::Fold => {
                if l.can_fold {
                    Action::Fold
                } else {
                    Action::Check
                }
            }
            Action::Check | Action::Call => {
                if l.can_check {
                    Action::Check
                } else {
                    Action::Call
                }
            }
            Action::RaiseTo(t) => {
                if !l.can_raise {
                    if l.can_check {
                        Action::Check
                    } else {
                        Action::Call
                    }
                } else if t as i64 == real_legal.max_raise_to as i64 {
                    Action::RaiseTo(l.max_raise_to)
                } else {
                    Action::RaiseTo(self.sc(t as i64).clamp(l.min_raise_to, l.max_raise_to))
                }
            }
        }
    }

    fn apply_event(&mut self, i: usize, ev: &ActionIn) -> Result<(), ApiError> {
        let at = |code: &'static str, msg: String| ApiError::invalid(code, format!("history[{i}]: {msg}")).with(json!({"index": i}));
        let Some(kind) = parse_kind(&ev.kind) else {
            return Err(at("unknown_action_type", format!("unknown action type {:?} (expected fold|check|call|bet|raise|all_in)", ev.kind)));
        };
        if matches!(kind, Kind::PostSb | Kind::PostBb) {
            return Err(at("bad_post", "blind posts are only allowed as the first history entries".into()));
        }
        if self.real.is_terminal() {
            return Err(at("action_after_hand_end", format!("seat {} acts after the hand has ended", ev.seat)));
        }
        let Some(ri) = (0..=MAX_SEAT).contains(&ev.seat).then(|| self.order.iter().position(|&s| s as i64 == ev.seat)).flatten() else {
            return Err(at("unknown_seat", format!("seat {} is not dealt into this hand", ev.seat)));
        };
        let expect = self.real.to_act().unwrap();
        if expect != ri {
            return Err(at("out_of_turn", format!("seat {} acts but seat {} is to act", ev.seat, self.order[expect]))
                .with(json!({"index": i, "expected_seat": self.order[expect], "got_seat": ev.seat})));
        }
        if let Some(s) = &ev.street {
            match parse_street(s) {
                Some(st) if st == self.real.street() => {}
                Some(st) => {
                    return Err(at("street_mismatch", format!("event says {} but the hand is on the {}", street_name(st), street_name(self.real.street()))))
                }
                None => return Err(at("street_mismatch", format!("unknown street {s:?}"))),
            }
        }
        let l = self.real.legal();
        let bet = self.real.street_bet(ri) as i64;
        let ra = match kind {
            Kind::PostSb | Kind::PostBb => unreachable!("handled above"),
            Kind::Fold => {
                if l.can_fold {
                    Action::Fold
                } else {
                    self.notes.push(format!("history[{i}]: fold while check is free, treated as check"));
                    Action::Check
                }
            }
            Kind::Check => {
                if !l.can_check {
                    return Err(at("illegal_action", format!("check while facing a bet (call {} to continue)", l.call_amount)));
                }
                Action::Check
            }
            Kind::Call => {
                let to = bet + l.call_amount as i64;
                if let Some(t) = ev.to {
                    if t != to && !(l.can_check && t == 0) {
                        return Err(at("bad_amount", format!("call to {t}, but calling is to {to}")).with(json!({"index": i, "call_to": to})));
                    }
                }
                if l.can_check {
                    Action::Check
                } else {
                    Action::Call
                }
            }
            Kind::Bet | Kind::Raise => {
                if kind == Kind::Bet && self.real.cur_bet() > 0 {
                    return Err(at("illegal_action", "bet when a bet is already open; use raise".into()));
                }
                let Some(to) = ev.to else { return Err(at("missing_amount", "bet/raise needs `to`".into())) };
                if !l.can_raise {
                    return Err(at("illegal_action", "this seat cannot raise here (no one left to act against, betting not reopened, or no chips)".into()));
                }
                if to < l.min_raise_to as i64 || to > l.max_raise_to as i64 {
                    return Err(at("bad_amount", format!("raise to {to} outside [{}, {}]", l.min_raise_to, l.max_raise_to))
                        .with(json!({"index": i, "min_to": l.min_raise_to, "max_to": l.max_raise_to})));
                }
                Action::RaiseTo(to as i32)
            }
            Kind::AllIn => {
                if l.can_raise {
                    if let Some(t) = ev.to {
                        if t != l.max_raise_to as i64 {
                            return Err(at("bad_amount", format!("all_in to {t}, but the stack goes to {}", l.max_raise_to)).with(json!({"index": i, "max_to": l.max_raise_to})));
                        }
                    }
                    Action::RaiseTo(l.max_raise_to)
                } else if l.can_check {
                    Action::Check
                } else {
                    Action::Call
                }
            }
        };

        // scaled twin
        let si = self.si(ri);
        let sa = self.to_scaled(ra, &l);
        let step_r = self.real.apply(ra);
        let step_s = if self.desync.is_none() && self.scaled.to_act() == Some(si) && !self.scaled.is_terminal() {
            Some(self.push_scaled(si, sa))
        } else {
            self.desync.get_or_insert_with(|| format!("history[{i}]: scaled engine out of step"));
            None
        };
        if let Some(s) = step_s {
            if s != step_r {
                self.desync = Some(format!("history[{i}]: scaled engine step {s:?} vs real {step_r:?}"));
            }
        }
        if let Step::NewStreet(st) = step_r {
            let need = st.board_len();
            if self.board.len() < need {
                return Err(at(
                    "board_mismatch",
                    format!("the history reaches the {} (needs {need} board cards) but the board has {}", street_name(st), self.board.len()),
                ));
            }
            if self.desync.is_none() {
                if let Some(p) = self.player.as_mut() {
                    use bench::policy::Player;
                    p.observe_board(&self.board[..need]);
                }
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------ outputs (real chips)

    pub fn hero_legal(&self) -> Legal {
        self.real.legal()
    }

    pub fn legal_out(&self) -> LegalOut {
        let l = self.real.legal();
        let bet = self.real.street_bet(self.hero_ri) as i64;
        let stack = self.real.stack(self.hero_ri) as i64;
        LegalOut {
            fold: l.can_fold,
            check: l.can_check,
            call: (!l.can_check).then(|| CallLegal { to: bet + l.call_amount as i64, added: l.call_amount as i64, all_in: l.call_amount as i64 >= stack }),
            raise: l.can_raise.then(|| RaiseLegal { min_to: l.min_raise_to as i64, max_to: l.max_raise_to as i64 }),
        }
    }

    /// Safest legal action: check, else fold.
    pub fn passive(&self) -> Action {
        if self.real.legal().can_check {
            Action::Check
        } else {
            Action::Fold
        }
    }

    pub fn is_legal(&self, a: Action) -> bool {
        let l = self.real.legal();
        match a {
            Action::Fold => l.can_fold,
            Action::Check => l.can_check,
            Action::Call => !l.can_check,
            Action::RaiseTo(t) => l.can_raise && t >= l.min_raise_to && t <= l.max_raise_to,
        }
    }

    /// A scaled-game action -> the closest legal real action.
    pub fn to_real(&self, a: Action) -> Action {
        let l = self.real.legal();
        let passive_or_call = || if l.can_check { Action::Check } else { Action::Call };
        match a {
            Action::Fold => {
                if l.can_fold {
                    Action::Fold
                } else {
                    Action::Check
                }
            }
            Action::Check | Action::Call => passive_or_call(),
            Action::RaiseTo(t) => {
                if !l.can_raise {
                    return passive_or_call();
                }
                let sl = self.scaled.legal();
                let r = if sl.can_raise && t == sl.max_raise_to { l.max_raise_to as i64 } else { (t as f64 / self.scale).round() as i64 };
                Action::RaiseTo(r.clamp(l.min_raise_to as i64, l.max_raise_to as i64) as i32)
            }
        }
    }

    pub fn describe(&self, a: Action) -> ActionOut {
        let l = self.real.legal();
        let bet = self.real.street_bet(self.hero_ri) as i64;
        match a {
            Action::Fold => ActionOut { kind: "fold", to: None, added: 0, all_in: false },
            Action::Check => ActionOut { kind: "check", to: None, added: 0, all_in: false },
            Action::Call => ActionOut {
                kind: "call",
                to: Some(bet + l.call_amount as i64),
                added: l.call_amount as i64,
                all_in: l.call_amount == self.real.stack(self.hero_ri),
            },
            Action::RaiseTo(t) => ActionOut { kind: "raise", to: Some(t as i64), added: t as i64 - bet, all_in: t == l.max_raise_to },
        }
    }

    /// Effective-stack guard (`--max-stack-bb`): never commit more than `cap` chips in one hand.
    /// Same rule as `arena/openpoker/src/mapping.rs::cap_action`: a raise/all-in beyond the cap becomes a
    /// raise to exactly `cap - prior` if that is a legal raise, else a call, else a check.
    pub fn cap_guard(&self, a: Action, cap: i64) -> Option<(Action, String)> {
        let Action::RaiseTo(t) = a else { return None };
        let ri = self.hero_ri;
        let total = self.real.contributed(ri) as i64;
        let bet = self.real.street_bet(ri) as i64;
        let prior = total - bet;
        if prior + t as i64 <= cap {
            return None;
        }
        let l = self.real.legal();
        let r = cap - prior;
        let new = if l.can_raise && r >= l.min_raise_to as i64 {
            Action::RaiseTo(r.min(l.max_raise_to as i64) as i32)
        } else if !l.can_check {
            Action::Call
        } else {
            Action::Check
        };
        Some((new, format!("stack guard: raise to {t} would commit {} chips (> cap {cap}); played {}", prior + t as i64, self.describe(new).kind)))
    }

    pub fn view(&self) -> View<'_> {
        View { cfg: &self.scaled_cfg, state: &self.scaled, board: &self.board, history: &self.history, seat: self.hero_si }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> ActRequest {
        serde_json::from_value(json!({
            "small_blind": 50, "big_blind": 100, "button_seat": 3, "hero_seat": 0,
            "hole_cards": ["Ah", "Kd"],
            "players": (0..6).map(|s| json!({"seat": s, "stack_start": 10000})).collect::<Vec<_>>(),
            "history": [
                {"seat": 4, "type": "post_sb", "to": 50}, {"seat": 5, "type": "post_bb", "to": 100},
                {"seat": 0, "type": "raise", "to": 250}
            ]
        }))
        .unwrap()
    }

    fn err(r: &ActRequest) -> ApiError {
        match Hand::replay(r, None, 1) {
            Ok(_) => panic!("expected an error"),
            Err(e) => e,
        }
    }

    #[test]
    fn replays_a_clean_history() {
        let mut r = base();
        r.history = vec![
            serde_json::from_value(json!({"seat": 4, "type": "post_sb", "to": 50})).unwrap(),
            serde_json::from_value(json!({"seat": 5, "type": "post_bb", "to": 100})).unwrap(),
            serde_json::from_value(json!({"seat": 0, "type": "raise", "to": 250})).unwrap(),
            serde_json::from_value(json!({"seat": 1, "type": "fold"})).unwrap(),
            serde_json::from_value(json!({"seat": 2, "type": "fold"})).unwrap(),
            serde_json::from_value(json!({"seat": 3, "type": "call", "to": 250})).unwrap(),
            serde_json::from_value(json!({"seat": 4, "type": "fold"})).unwrap(),
            serde_json::from_value(json!({"seat": 5, "type": "call"})).unwrap(),
        ];
        // SB..BTN = seats 4,5,0,1,2,3; the round closes -> the flop needs board cards
        assert_eq!(err(&r).code, "board_mismatch");
        r.board = vec!["2c".into(), "7d".into(), "9h".into()];
        // postflop the first live seat left of the button acts: seat 5 (the BB), not the hero
        let e = err(&r);
        assert_eq!(e.code, "not_hero_turn");
        assert_eq!(e.details.unwrap()["to_act_seat"], 5);
    }

    #[test]
    fn hero_to_act_with_legal_set_in_real_chips() {
        let mut r = base();
        r.history.clear();
        // utg = seat 0 (hero): facing the 100 blind
        let h = Hand::replay(&r, None, 1).unwrap();
        let l = h.legal_out();
        assert!(l.fold && !l.check);
        assert_eq!(l.call.as_ref().unwrap().to, 100);
        let rl = l.raise.unwrap();
        assert_eq!((rl.min_to, rl.max_to), (200, 10000));
    }

    #[test]
    fn rejects_inconsistent_histories() {
        let b = base(); // seat 0 raised already, so now seat 1 is to act, hero is 0
        assert_eq!(err(&b).code, "not_hero_turn");
        let mut r = b.clone();
        r.history[2].to = Some(150); // below min raise (200)
        assert_eq!(err(&r).code, "bad_amount");
        let mut r = b.clone();
        r.history[2].seat = 1;
        assert_eq!(err(&r).code, "out_of_turn");
        let mut r = b.clone();
        r.history[2].kind = "check".into();
        assert_eq!(err(&r).code, "illegal_action");
        let mut r = b.clone();
        r.hole_cards = vec!["Ah".into(), "Ah".into()];
        assert_eq!(err(&r).code, "duplicate_card");
        let mut r = b.clone();
        r.board = vec!["2c".into(), "7d".into(), "9h".into()];
        r.history.clear();
        assert_eq!(err(&r).code, "board_mismatch");
        let mut r = b.clone();
        r.hole_cards[0] = "Xx".into();
        assert_eq!(err(&r).code, "invalid_card");
        let mut r = b.clone();
        r.history[0].seat = 3;
        assert_eq!(err(&r).code, "bad_post");
    }

    #[test]
    fn heads_up_button_is_small_blind() {
        let r: ActRequest = serde_json::from_value(json!({
            "small_blind": 50, "big_blind": 100, "button_seat": 2, "hero_seat": 2,
            "hole_cards": ["Ah", "Kd"],
            "players": [{"seat": 2, "stack_start": 10000}, {"seat": 5, "stack_start": 10000}, {"seat": 0, "stack_start": 10000, "in_hand": false}],
            "history": []
        }))
        .unwrap();
        let h = Hand::replay(&r, None, 1).unwrap();
        assert_eq!(h.order, vec![2, 5]); // button = SB acts first preflop
        assert!(h.legal_out().call.is_some());
    }

    #[test]
    fn embeds_short_handed_and_scales_chips() {
        // 4 players, bb = 20 (scale x5), button 1; order after button: 2,3,0,1
        let r: ActRequest = serde_json::from_value(json!({
            "small_blind": 10, "big_blind": 20, "button_seat": 1, "hero_seat": 0,
            "hole_cards": ["Qh", "Qd"],
            "players": [{"seat": 0, "stack_start": 2000}, {"seat": 1, "stack_start": 2000}, {"seat": 2, "stack_start": 2000}, {"seat": 3, "stack_start": 2000}],
            "history": []
        }))
        .unwrap();
        let h = Hand::replay(&r, None, 1).unwrap();
        assert_eq!(h.order, vec![2, 3, 0, 1]);
        assert_eq!(h.prefold, 2);
        assert_eq!(h.scaled_cfg.players, 6);
        assert_eq!(h.scaled_cfg.big_blind, 100);
        assert_eq!(h.scaled.to_act(), Some(h.hero_si));
        assert!(h.desync.is_none());
        // a 3x open (60 real chips) maps to 300 scaled and back
        let back = h.to_real(Action::RaiseTo(300));
        assert_eq!(back, Action::RaiseTo(60));
    }

    #[test]
    fn stack_guard_caps_a_deep_shove() {
        let r: ActRequest = serde_json::from_value(json!({
            "small_blind": 50, "big_blind": 100, "button_seat": 0, "hero_seat": 3,
            "hole_cards": ["Ah", "Ad"],
            "players": (0..6).map(|s| json!({"seat": s, "stack_start": 100000})).collect::<Vec<_>>(),
            "history": []
        }))
        .unwrap();
        let h = Hand::replay(&r, None, 1).unwrap();
        // all-in (100000 = 1000bb) with a 150bb cap (15000)
        let (a, note) = h.cap_guard(Action::RaiseTo(100000), 15000).unwrap();
        assert_eq!(a, Action::RaiseTo(15000));
        assert!(note.contains("stack guard"));
        assert!(h.cap_guard(Action::RaiseTo(12000), 15000).is_none());
        assert!(h.is_legal(a));
    }
}
