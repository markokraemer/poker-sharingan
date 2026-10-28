//! One hand of NLHE as an event-producing state machine on top of `nlhe::State`.
//!
//! The rules live entirely in the `nlhe` engine; this module maps physical seats to engine seats,
//! deals from a given deck, validates client actions against `State::legal()` and renders every
//! payload of TABLE_API.md section 5 (`hand_start`, `hole_cards`, `action`, `street`, `your_turn`,
//! `hand_end`) plus the `HandHistory` of section 8. It is synchronous and does no I/O.
//!
//! Engine seat `i` (0 = SB ... last = button; heads-up: 0 = button/SB) maps to the physical seat
//! `order[i]`. Sequence numbers: `action`, `street`, `your_turn` and `hand_end` events share one
//! gap-free per-hand counter starting at 1 (blinds are 1 and 2); `hand_start` and `hole_cards`
//! are unsequenced.

use crate::deck;
use crate::error::{ApiError, Res};
use crate::model::*;
use crate::util::iso;
use bench::policy::Event;
use cards::{bit, card_str, category, evaluate, Card};
use nlhe::{Action, Deal, GameConfig, State, Step, Street, MAX_PLAYERS};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct SeatInfo {
    pub seat: usize,
    pub player_id: String,
    pub name: String,
    pub kind: String,
    pub stack: i64,
}

#[derive(Clone, Debug)]
pub struct HandSetup {
    pub table_id: String,
    pub hand_id: String,
    pub match_id: Option<String>,
    pub hand_no: u32,
    pub deal_no: Option<u32>,
    pub rotation: Option<u32>,
    pub cfg: TableConfig,
    pub button_seat: usize,
    /// Seats dealt into this hand (2..=6).
    pub seats: Vec<SeatInfo>,
    /// Seated but not dealt in (reported as `in_hand: false`).
    pub sitting_out: Vec<SeatInfo>,
    pub deck: Vec<Card>,
    pub salt: String,
    pub started_ms: u64,
    /// Benchmark fast path: build no JSON for events the recipients will not get.
    pub lean: bool,
}

/// A server event of this hand before it is wrapped in an envelope.
#[derive(Clone, Debug)]
pub struct HandEvent {
    pub kind: &'static str,
    pub seq: Option<u32>,
    /// Physical seat the event is private to; `None` = broadcast.
    pub to: Option<usize>,
    pub data: Value,
}

#[derive(Clone, Debug)]
pub struct ActReq {
    pub kind: String,
    pub to: Option<i64>,
}

pub struct Applied {
    pub events: Vec<HandEvent>,
    pub action: Action,
    /// State before the action (for `Player::observe`).
    pub pre: State,
    pub idx: usize,
    /// Board lengths revealed by this action (for `Player::observe_board`).
    pub reveals: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct PendingTurn {
    pub decision_id: String,
    pub idx: usize,
    pub seat: usize,
    pub event: HandEvent,
}

pub struct HandRunner {
    pub setup: HandSetup,
    pub order: Vec<usize>,
    pub info: Vec<SeatInfo>,
    pub ecfg: GameConfig,
    pub st: State,
    pub deal: Deal,
    pub board_len: usize,
    pub ehist: Vec<Event>,
    actions: Vec<ActionEvent>,
    seq: u32,
    pub pending: Option<PendingTurn>,
    decision_times: BTreeMap<String, u64>,
    pub anomalies: Vec<String>,
    last_decision: u32,
}

fn street_name(s: Street) -> &'static str {
    ["preflop", "flop", "turn", "river"][s.idx()]
}

fn street_for_len(len: usize) -> &'static str {
    match len {
        3 => "flop",
        4 => "turn",
        _ => "river",
    }
}

fn cstr(c: Card) -> String {
    card_str(c)
}

fn pair(h: [Card; 2]) -> [String; 2] {
    [cstr(h[0]), cstr(h[1])]
}

impl HandRunner {
    /// Build the hand and its opening events (`hand_start`, `hole_cards`, blinds, and any run-out
    /// when the blinds already put everyone all-in).
    pub fn new(setup: HandSetup) -> Res<(HandRunner, Vec<HandEvent>)> {
        let n = setup.seats.len();
        if !(2..=MAX_PLAYERS).contains(&n) {
            return Err(ApiError::internal("hand needs 2..6 players"));
        }
        if setup.deck.len() != 52 {
            return Err(ApiError::internal("deck must have 52 cards"));
        }
        let max_seats = setup.cfg.max_seats as usize;
        let occupied: Vec<usize> = setup.seats.iter().map(|s| s.seat).collect();
        if !occupied.contains(&setup.button_seat) {
            return Err(ApiError::internal("button must be an occupied seat"));
        }
        let order: Vec<usize> = if n == 2 {
            let other = *occupied.iter().find(|&&s| s != setup.button_seat).unwrap();
            vec![setup.button_seat, other]
        } else {
            (1..=max_seats)
                .map(|k| (setup.button_seat + k) % max_seats)
                .filter(|s| occupied.contains(s))
                .collect()
        };
        let info: Vec<SeatInfo> = order
            .iter()
            .map(|&s| setup.seats.iter().find(|x| x.seat == s).unwrap().clone())
            .collect();
        let stacks: Vec<i32> = info.iter().map(|s| s.stack as i32).collect();
        let ecfg = GameConfig {
            players: n,
            stack: setup.cfg.starting_stack as i32,
            small_blind: setup.cfg.small_blind as i32,
            big_blind: setup.cfg.big_blind as i32,
            stacks: None,
        }
        .with_stacks(&stacks);
        let st = State::new(&ecfg);
        let d = &setup.deck;
        let mut holes = [[0u8; 2]; MAX_PLAYERS];
        for (i, h) in holes.iter_mut().enumerate().take(n) {
            *h = [d[i], d[n + i]];
        }
        let board = [d[2 * n + 1], d[2 * n + 2], d[2 * n + 3], d[2 * n + 5], d[2 * n + 7]];
        let deal = Deal { holes, board };
        let mut r = HandRunner {
            setup,
            order,
            info,
            ecfg,
            st,
            deal,
            board_len: 0,
            ehist: Vec::new(),
            actions: Vec::new(),
            seq: 0,
            pending: None,
            decision_times: BTreeMap::new(),
            anomalies: Vec::new(),
            last_decision: 0,
        };
        let mut ev = Vec::new();
        if !r.setup.lean {
            ev.push(r.hand_start_event());
        }
        for i in 0..n {
            if r.setup.lean {
                break;
            }
            ev.push(HandEvent {
                kind: "hole_cards",
                seq: None,
                to: Some(r.order[i]),
                data: json!({"hand_id": r.setup.hand_id, "seat": r.order[i], "cards": pair(r.deal.holes[i])}),
            });
        }
        r.post_blinds(&mut ev);
        if r.st.is_terminal() {
            r.runout(&mut ev, &mut Vec::new());
        }
        Ok((r, ev))
    }

    // ---- small accessors ----
    pub fn hand_id(&self) -> &str {
        &self.setup.hand_id
    }
    pub fn n(&self) -> usize {
        self.info.len()
    }
    pub fn seat_of(&self, idx: usize) -> usize {
        self.order[idx]
    }
    pub fn idx_of_seat(&self, seat: usize) -> Option<usize> {
        self.order.iter().position(|&s| s == seat)
    }
    pub fn is_over(&self) -> bool {
        self.st.is_terminal()
    }
    pub fn to_act_idx(&self) -> Option<usize> {
        self.st.to_act()
    }
    pub fn board(&self) -> &[Card] {
        &self.deal.board[..self.board_len]
    }
    fn next_seq(&mut self) -> u32 {
        self.seq += 1;
        self.seq
    }
    fn aid(&self, seq: u32) -> String {
        format!("a-{}-{}", self.setup.hand_id, seq)
    }
    fn sb_seat(&self) -> usize {
        self.order[0]
    }
    fn bb_seat(&self) -> usize {
        self.order[1]
    }

    fn hand_start_event(&self) -> HandEvent {
        let s = &self.setup;
        let mut players: Vec<Value> = s
            .seats
            .iter()
            .map(|p| (p, true))
            .chain(s.sitting_out.iter().map(|p| (p, false)))
            .map(|(p, inh)| {
                json!({"seat": p.seat, "player_id": p.player_id, "name": p.name, "stack_start": p.stack, "in_hand": inh})
            })
            .collect();
        players.sort_by_key(|p| p["seat"].as_u64());
        let ds = s.deal_no;
        HandEvent {
            kind: "hand_start",
            seq: None,
            to: None,
            data: json!({
                "hand_id": s.hand_id, "hand_no": s.hand_no, "match_id": s.match_id,
                "deal_no": ds, "rotation": s.rotation,
                "button_seat": s.button_seat, "sb_seat": self.sb_seat(), "bb_seat": self.bb_seat(),
                "small_blind": s.cfg.small_blind, "big_blind": s.cfg.big_blind, "ante": 0,
                "players": players,
                "deck_commitment": deck::commitment(&s.deck, &s.salt),
            }),
        }
    }

    fn mk_action(
        &mut self,
        street: &str,
        idx: usize,
        kind: &str,
        to: i64,
        added: i64,
        all_in: bool,
        forced: bool,
        timeout: bool,
        stack: i64,
        pot: i64,
        current_bet: i64,
        min_raise_to: i64,
        next: Option<usize>,
    ) -> ActionEvent {
        let seq = self.next_seq();
        let ev = ActionEvent {
            hand_id: self.setup.hand_id.clone(),
            seq,
            action_id: self.aid(seq),
            street: street.to_string(),
            seat: self.order[idx] as u8,
            kind: kind.to_string(),
            to,
            added,
            all_in,
            forced,
            timeout,
            stack,
            pot,
            current_bet,
            min_raise_to,
            next_to_act: next.map(|i| self.order[i] as u8),
        };
        self.actions.push(ev.clone());
        ev
    }

    fn action_he(ev: &ActionEvent) -> HandEvent {
        HandEvent { kind: "action", seq: Some(ev.seq), to: None, data: serde_json::to_value(ev).unwrap() }
    }

    fn post_blinds(&mut self, ev: &mut Vec<HandEvent>) {
        let (sb, bb) = (self.setup.cfg.small_blind, self.setup.cfg.big_blind);
        let s0 = self.info[0].stack;
        let s1 = self.info[1].stack;
        let sb_add = sb.min(s0);
        let bb_add = bb.min(s1);
        let a = self.mk_action(
            "preflop", 0, "post_sb", sb_add, sb_add, sb_add == s0, true, false,
            s0 - sb_add, sb_add, sb_add, bb, Some(1),
        );
        if !self.setup.lean {
            ev.push(Self::action_he(&a));
        }
        let cur = sb_add.max(bb_add);
        let next = self.st.to_act();
        let b = self.mk_action(
            "preflop", 1, "post_bb", bb_add, bb_add, bb_add == s1, true, false,
            s1 - bb_add, sb_add + bb_add, cur, cur + self.st.last_raise() as i64, next,
        );
        if !self.setup.lean {
            ev.push(Self::action_he(&b));
        }
    }

    // ---- pots ----
    /// Main and side pots as (amount, eligible engine seats).
    fn pot_levels(&self) -> Vec<(i64, Vec<usize>)> {
        let n = self.n();
        let live: Vec<usize> = (0..n).filter(|&i| !self.st.is_folded(i)).collect();
        let tot = |i: usize| self.st.contributed(i) as i64;
        let mut levels: Vec<i64> = live.iter().map(|&i| tot(i)).collect();
        levels.sort_unstable();
        levels.dedup();
        let mut prev = 0;
        let mut out = Vec::new();
        for lv in levels {
            if lv == prev {
                continue;
            }
            let amount: i64 = (0..n).map(|i| tot(i).min(lv) - tot(i).min(prev)).sum();
            let elig: Vec<usize> = live.iter().copied().filter(|&i| tot(i) >= lv).collect();
            out.push((amount, elig));
            prev = lv;
        }
        out
    }

    fn pots_json(&self) -> Value {
        Value::Array(
            self.pot_levels()
                .into_iter()
                .map(|(a, e)| {
                    let mut seats: Vec<usize> = e.iter().map(|&i| self.order[i]).collect();
                    seats.sort_unstable();
                    json!({"amount": a, "eligible_seats": seats})
                })
                .collect(),
        )
    }

    fn status_of(&self, i: usize) -> &'static str {
        if self.st.is_folded(i) {
            "folded"
        } else if self.st.is_all_in(i) {
            "all_in"
        } else {
            "active"
        }
    }

    fn players_json(&self) -> Value {
        let mut v: Vec<(usize, Value)> = (0..self.n())
            .map(|i| {
                (
                    self.order[i],
                    json!({
                        "seat": self.order[i], "player_id": self.info[i].player_id, "name": self.info[i].name,
                        "stack_start": self.info[i].stack, "stack": self.st.stack(i),
                        "street_contrib": self.st.street_bet(i), "hand_contrib": self.st.contributed(i),
                        "status": self.status_of(i),
                    }),
                )
            })
            .collect();
        v.sort_by_key(|x| x.0);
        Value::Array(v.into_iter().map(|x| x.1).collect())
    }

    /// Public hand state for `GET /v1/tables/{id}` (no hole cards).
    pub fn public_state(&self) -> Value {
        let street = if self.is_over() { "showdown" } else { street_name(self.st.street()) };
        json!({
            "hand_id": self.setup.hand_id, "hand_no": self.setup.hand_no, "match_id": self.setup.match_id,
            "deal_no": self.setup.deal_no, "rotation": self.setup.rotation,
            "street": street,
            "board": self.board().iter().map(|&c| cstr(c)).collect::<Vec<_>>(),
            "button_seat": self.setup.button_seat, "sb_seat": self.sb_seat(), "bb_seat": self.bb_seat(),
            "small_blind": self.setup.cfg.small_blind, "big_blind": self.setup.cfg.big_blind,
            "pot": self.st.pot(), "pots": self.pots_json(),
            "to_act": self.st.to_act().map(|i| self.order[i]),
            "players": self.players_json(),
            "actions": self.actions.len(),
        })
    }

    pub fn hole_cards_of_seat(&self, seat: usize) -> Option<[String; 2]> {
        self.idx_of_seat(seat).map(|i| pair(self.deal.holes[i]))
    }

    // ---- turns ----
    /// Create the `your_turn` event for the seat to act (allocates its seq and decision id).
    pub fn begin_turn(&mut self, deadline_ms: u64, bank_ms: u64) -> Option<&PendingTurn> {
        let idx = self.st.to_act()?;
        // `your_turn` carries the seq the next event will take but does not consume it, so the
        // seqs of all other events stay gap-free.
        let seq = self.seq + 1;
        self.last_decision += 1;
        let decision_id = format!("d-{}-{}", self.setup.hand_id, seq);
        let l = self.st.legal();
        let call = if l.can_check {
            Value::Null
        } else {
            json!({"to": self.st.street_bet(idx) + l.call_amount, "added": l.call_amount,
                   "all_in": l.call_amount >= self.st.stack(idx)})
        };
        let raise = if l.can_raise {
            json!({"min_to": l.min_raise_to, "max_to": l.max_raise_to})
        } else {
            Value::Null
        };
        let seat = self.order[idx];
        let data = json!({
            "hand_id": self.setup.hand_id, "seq": seq, "decision_id": decision_id, "seat": seat,
            "street": street_name(self.st.street()),
            "hole_cards": pair(self.deal.holes[idx]),
            "board": self.board().iter().map(|&c| cstr(c)).collect::<Vec<_>>(),
            "history": self.actions,
            "players": self.players_json(),
            "button_seat": self.setup.button_seat,
            "small_blind": self.setup.cfg.small_blind, "big_blind": self.setup.cfg.big_blind,
            "pot": self.st.pot(), "pots": self.pots_json(),
            "legal": {"fold": l.can_fold, "check": l.can_check, "call": call, "raise": raise},
            "deadline_ms": deadline_ms, "time_bank_ms": bank_ms,
        });
        self.pending = Some(PendingTurn {
            decision_id,
            idx,
            seat,
            event: HandEvent { kind: "your_turn", seq: Some(seq), to: Some(seat), data },
        });
        self.pending.as_ref()
    }

    /// Validate a client action against the legal set; the state is untouched on error.
    pub fn validate(&self, req: &ActReq) -> Res<Action> {
        let l = self.st.legal();
        let illegal = |m: &str| {
            ApiError::new(422, "illegal_action", m.to_string()).with(json!({"type": req.kind}))
        };
        Ok(match req.kind.as_str() {
            // Fold when check is free is converted to a check (TABLE_API.md 6.6).
            "fold" => {
                if l.can_fold {
                    Action::Fold
                } else {
                    Action::Check
                }
            }
            "check" => {
                if !l.can_check {
                    return Err(illegal("cannot check, there is a bet to call"));
                }
                Action::Check
            }
            "call" => {
                if l.can_check {
                    return Err(illegal("nothing to call, use check"));
                }
                Action::Call
            }
            "bet" | "raise" => {
                if !l.can_raise {
                    return Err(illegal("raising is not allowed here"));
                }
                if req.kind == "bet" && self.st.cur_bet() > 0 {
                    return Err(illegal("bet is only valid when there is no bet, use raise"));
                }
                let to = req.to.ok_or_else(|| {
                    ApiError::new(422, "bad_amount", "`to` is required for bet/raise")
                        .with(json!({"min_to": l.min_raise_to, "max_to": l.max_raise_to}))
                })?;
                if to < l.min_raise_to as i64 || to > l.max_raise_to as i64 {
                    return Err(ApiError::new(
                        422,
                        "bad_amount",
                        format!("`to` must be in [{}, {}]", l.min_raise_to, l.max_raise_to),
                    )
                    .with(json!({"min_to": l.min_raise_to, "max_to": l.max_raise_to, "to": to})));
                }
                Action::RaiseTo(to as i32)
            }
            _ => return Err(illegal("unknown action type")),
        })
    }

    /// Timeout policy: check if free, else fold.
    pub fn timeout_action(&self) -> Action {
        if self.st.legal().can_check {
            Action::Check
        } else {
            Action::Fold
        }
    }

    /// Apply an engine action for the seat to act. `think_ms` is recorded in the history.
    pub fn apply(&mut self, action: Action, timeout: bool, think_ms: u64) -> Applied {
        let idx = self.st.to_act().expect("apply on a live hand");
        let pre = self.st;
        let street = pre.street();
        let mut st = self.st;
        let step = st.apply(action);
        self.st = st;
        let added = (pre.stack(idx) - st.stack(idx)) as i64;
        let kind = match action {
            Action::Fold => "fold",
            Action::Check => "check",
            Action::Call => {
                if added == 0 {
                    "check"
                } else {
                    "call"
                }
            }
            Action::RaiseTo(_) => {
                if pre.cur_bet() == 0 {
                    "bet"
                } else {
                    "raise"
                }
            }
        };
        let to = match kind {
            "fold" | "check" => 0,
            _ => (pre.street_bet(idx) as i64) + added,
        };
        let all_in = kind != "fold" && st.is_all_in(idx);
        let next = st.to_act();
        let ev = self.mk_action(
            street_name(street), idx, kind, to, added, all_in, false, timeout,
            st.stack(idx) as i64, st.pot() as i64, st.cur_bet() as i64,
            (st.cur_bet() + st.last_raise()) as i64, next,
        );
        self.decision_times.insert(ev.action_id.clone(), think_ms);
        self.ehist.push(Event { seat: idx as u8, street, action });
        self.pending = None;
        let mut events = if self.setup.lean { Vec::new() } else { vec![Self::action_he(&ev)] };
        let mut reveals = Vec::new();
        match step {
            Step::Continue => {}
            Step::NewStreet(s) => {
                self.reveal(s.board_len(), &mut events);
                reveals.push(s.board_len());
            }
            Step::Terminal => {
                if st.showdown() {
                    self.runout(&mut events, &mut reveals);
                }
            }
        }
        Applied { events, action, pre, idx, reveals }
    }

    fn reveal(&mut self, to: usize, events: &mut Vec<HandEvent>) {
        let from = self.board_len;
        self.board_len = to;
        let seq = self.next_seq();
        if self.setup.lean {
            return;
        }
        events.push(HandEvent {
            kind: "street",
            seq: Some(seq),
            to: None,
            data: json!({
                "hand_id": self.setup.hand_id, "seq": seq, "street": street_for_len(to),
                "board": self.board().iter().map(|&c| cstr(c)).collect::<Vec<_>>(),
                "new_cards": self.deal.board[from..to].iter().map(|&c| cstr(c)).collect::<Vec<_>>(),
                "pot": self.st.pot(), "pots": self.pots_json(),
            }),
        });
    }

    fn runout(&mut self, events: &mut Vec<HandEvent>, reveals: &mut Vec<usize>) {
        if !self.st.showdown() {
            return;
        }
        while self.board_len < 5 {
            let to = if self.board_len == 0 { 3 } else { self.board_len + 1 };
            self.reveal(to, events);
            reveals.push(to);
        }
    }

    fn best5(&self, idx: usize) -> (String, Vec<String>) {
        let cs: Vec<Card> = self.deal.holes[idx].iter().chain(self.deal.board.iter()).copied().collect();
        let full = evaluate(cs.iter().fold(0, |m, &c| m | bit(c)));
        let mut best: Vec<Card> = Vec::new();
        for a in 0..7 {
            for b in a + 1..7 {
                let pick: Vec<Card> = (0..7).filter(|&k| k != a && k != b).map(|k| cs[k]).collect();
                if evaluate(pick.iter().fold(0, |m, &c| m | bit(c))) == full {
                    best = pick;
                    break;
                }
            }
            if !best.is_empty() {
                break;
            }
        }
        best.sort_by_key(|&c| std::cmp::Reverse(cards::rank_of(c)));
        let name = format!("{:?}", category(full)).to_lowercase();
        (name, best.into_iter().map(cstr).collect())
    }

    /// Pots with winners and shares, computed like the engine's `payoffs` (odd chips left of the button).
    fn pot_results(&mut self) -> Vec<PotResult> {
        let n = self.n();
        let levels = self.pot_levels();
        let live: Vec<usize> = (0..n).filter(|&i| !self.st.is_folded(i)).collect();
        let mut rank = [0u32; MAX_PLAYERS];
        if self.st.showdown() {
            let board = self.deal.board.iter().fold(0, |m, &c| m | bit(c));
            for &i in &live {
                rank[i] = evaluate(bit(self.deal.holes[i][0]) | bit(self.deal.holes[i][1]) | board);
            }
        }
        let first = if n == 2 { 1 } else { 0 };
        let mut out = Vec::new();
        for (amount, elig) in levels {
            let best = elig.iter().map(|&i| rank[i]).max().unwrap_or(0);
            let winners: Vec<usize> = (0..n)
                .map(|k| (first + k) % n)
                .filter(|i| elig.contains(i) && rank[*i] == best)
                .collect();
            let w = winners.len().max(1) as i64;
            let (share, odd) = (amount / w, (amount % w) as usize);
            let shares: Vec<i64> = (0..winners.len()).map(|k| share + (k < odd) as i64).collect();
            let mut es: Vec<u8> = elig.iter().map(|&i| self.order[i] as u8).collect();
            es.sort_unstable();
            out.push(PotResult {
                amount,
                eligible_seats: es,
                winners: winners.iter().map(|&i| self.order[i] as u8).collect(),
                share: shares,
            });
        }
        out
    }

    /// Settle a terminal hand: the `hand_end` event and the full history.
    pub fn finish(&mut self, want_end: bool) -> (Vec<HandEvent>, HandHistory) {
        assert!(self.st.is_terminal());
        let n = self.n();
        let mut events = Vec::new();
        self.runout(&mut events, &mut Vec::new());
        let payoffs = self.st.payoffs(&self.deal);
        let pots = self.pot_results();
        // Cross-check the per-pot split against the engine's payoffs.
        let mut net = vec![0i64; n];
        for i in 0..n {
            net[i] = -(self.st.contributed(i) as i64);
        }
        for p in &pots {
            for (w, s) in p.winners.iter().zip(&p.share) {
                let i = self.idx_of_seat(*w as usize).unwrap();
                net[i] += s;
            }
        }
        for i in 0..n {
            if net[i] != payoffs[i] as i64 {
                self.anomalies.push(format!("pot split mismatch at seat {}: {} vs engine {}", self.order[i], net[i], payoffs[i]));
            }
        }
        let results: Vec<ResultRow> = {
            let mut v: Vec<ResultRow> = (0..n)
                .map(|i| ResultRow {
                    seat: self.order[i] as u8,
                    player_id: self.info[i].player_id.clone(),
                    net: payoffs[i] as i64,
                    stack_end: self.info[i].stack + payoffs[i] as i64,
                })
                .collect();
            v.sort_by_key(|r| r.seat);
            v
        };
        if results.iter().map(|r| r.net).sum::<i64>() != 0 {
            self.anomalies.push("sum(net) != 0".into());
        }
        let showdown = self.st.showdown();
        let mut sd = Vec::new();
        if showdown && want_end {
            let mut live: Vec<usize> = (0..n).filter(|&i| !self.st.is_folded(i)).collect();
            live.sort_by_key(|&i| self.order[i]);
            for i in live {
                let (rank, best5) = self.best5(i);
                sd.push(json!({"seat": self.order[i], "cards": pair(self.deal.holes[i]), "shown": true,
                               "hand_rank": rank, "best5": best5}));
            }
        }
        let mut hole_cards: BTreeMap<u8, [String; 2]> = BTreeMap::new();
        for i in 0..n {
            hole_cards.insert(self.order[i] as u8, pair(self.deal.holes[i]));
        }
        let seq = self.next_seq();
        let ended_by = if showdown { "showdown" } else { "fold" };
        let runout: Vec<String> = self.deal.board.iter().map(|&c| cstr(c)).collect();
        let deck_strs: Vec<String> = self.setup.deck.iter().map(|&c| cstr(c)).collect();
        if want_end {
        events.push(HandEvent {
            kind: "hand_end",
            seq: Some(seq),
            to: None,
            data: json!({
                "hand_id": self.setup.hand_id, "seq": seq,
                "board": self.board().iter().map(|&c| cstr(c)).collect::<Vec<_>>(),
                "runout": runout,
                "showdown": sd,
                "hole_cards": hole_cards,
                "pots": pots, "results": results,
                "deck_reveal": {"deck": deck_strs, "salt": self.setup.salt},
                "ended_by": ended_by,
            }),
        });
        }
        let s = &self.setup;
        let mut players: Vec<HistPlayer> = self
            .info
            .iter()
            .map(|p| HistPlayer {
                seat: p.seat as u8,
                player_id: p.player_id.clone(),
                name: p.name.clone(),
                kind: p.kind.clone(),
                stack_start: p.stack,
            })
            .collect();
        players.sort_by_key(|p| p.seat);
        let ended_ms = crate::util::now_ms();
        let hist = HandHistory {
            hand_id: s.hand_id.clone(),
            hand_no: s.hand_no,
            match_id: s.match_id.clone(),
            table_id: s.table_id.clone(),
            deal_no: s.deal_no,
            rotation: s.rotation,
            started_at: iso(s.started_ms),
            ended_at: iso(ended_ms),
            config: HistConfig {
                small_blind: s.cfg.small_blind,
                big_blind: s.cfg.big_blind,
                ante: 0,
                starting_stack: s.cfg.starting_stack,
                stack_mode: format!("{:?}", s.cfg.stack_mode).to_lowercase(),
            },
            button_seat: s.button_seat as u8,
            sb_seat: self.sb_seat() as u8,
            bb_seat: self.bb_seat() as u8,
            players,
            hole_cards,
            board: self.board().iter().map(|&c| cstr(c)).collect(),
            runout: self.deal.board.iter().map(|&c| cstr(c)).collect(),
            deck: deck_strs_of(&s.deck),
            deck_salt: s.salt.clone(),
            actions: self.actions.clone(),
            decision_times_ms: self.decision_times.clone(),
            pots,
            results,
            ended_by: ended_by.into(),
            anomalies: self.anomalies.clone(),
        };
        (events, hist)
    }
}

fn deck_strs_of(d: &[Card]) -> Vec<String> {
    d.iter().map(|&c| cstr(c)).collect()
}

/// Rebuild the engine view of a stored hand for replay (AIVAT): config, deal and the list of
/// voluntary actions per engine seat. Returns `(cfg, deal, order, actions)`.
pub fn replay_parts(h: &HandHistory) -> Res<(GameConfig, Deal, Vec<usize>, Vec<Action>)> {
    let n = h.players.len();
    let max_seats = 6usize;
    let occupied: Vec<usize> = h.players.iter().map(|p| p.seat as usize).collect();
    let button = h.button_seat as usize;
    let order: Vec<usize> = if n == 2 {
        let other = *occupied.iter().find(|&&s| s != button).ok_or_else(|| ApiError::internal("bad history"))?;
        vec![button, other]
    } else {
        (1..=max_seats).map(|k| (button + k) % max_seats).filter(|s| occupied.contains(s)).collect()
    };
    let stacks: Vec<i32> = order
        .iter()
        .map(|s| h.players.iter().find(|p| p.seat as usize == *s).map(|p| p.stack_start as i32).unwrap_or(0))
        .collect();
    let ecfg = GameConfig {
        players: n,
        stack: h.config.starting_stack as i32,
        small_blind: h.config.small_blind as i32,
        big_blind: h.config.big_blind as i32,
        stacks: None,
    }
    .with_stacks(&stacks);
    let deck: Vec<Card> = h
        .deck
        .iter()
        .map(|s| cards::parse_card(s).ok_or_else(|| ApiError::internal("bad card")))
        .collect::<Res<_>>()?;
    let mut holes = [[0u8; 2]; MAX_PLAYERS];
    for (i, hh) in holes.iter_mut().enumerate().take(n) {
        *hh = [deck[i], deck[n + i]];
    }
    let deal = Deal { holes, board: [deck[2 * n + 1], deck[2 * n + 2], deck[2 * n + 3], deck[2 * n + 5], deck[2 * n + 7]] };
    let actions = h
        .actions
        .iter()
        .filter(|a| !a.forced)
        .map(|a| match a.kind.as_str() {
            "fold" => Action::Fold,
            "check" => Action::Check,
            "call" => Action::Call,
            _ => Action::RaiseTo(a.to as i32),
        })
        .collect();
    Ok((ecfg, deal, order, actions))
}
