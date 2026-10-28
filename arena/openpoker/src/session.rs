//! The socket-free session state machine: server messages in, client messages out.
//!
//! One `Session` plays any number of hands at whatever tables the matchmaker gives us. All
//! network IO lives in [`crate::live`]; this module is driven by recorded messages in tests.
//!
//! # Per-hand flow
//! `hand_start` opens a context. `player_action` / `community_cards` / `hole_cards` are
//! buffered until our first `your_turn`; then the engine replay is built (start stacks derived
//! from the server's current stacks, see `build_sim`), the policy is queried, the chosen
//! action is mapped to a legal Open Poker action and sent. Later actions are applied as they
//! arrive. `hand_result` closes the hand and emits the `hand` trace record.
//!
//! # Failure policy
//! Anything that makes the replay untrustworthy (unplaceable action, hero not to act per the
//! engine, policy error) marks the hand `desync` and answers with the safest legal action
//! (check, else fold). The hand is still recorded. A rejected action is retried once with the
//! safe action. We never sit out.

use crate::analyze::{cards_to_strs, shown_from_value, HandAct, HandPlayer, HandRecord};
use crate::layout::{engine_order, Layout};
use crate::mapping::{cap_action, is_legal, map_action, passive, Fidelity, OpAction};
use crate::proto::{self, ActionObs, ObsKind, PlayerInfo, ValidSet};
use crate::sim::Sim;
use crate::trace::{now_ms, TraceSink};
use bench::mix;
use bench::policy::PolicyFactory;
use cards::{Card, Rng};
use nlhe::Action;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HuMode {
    /// Heads-up hands are played by the fallback policy (default; a 6-max blueprint has no
    /// heads-up strategy).
    Fallback,
    /// Heads-up hands go to the main policy on a native 2-player engine (out of distribution
    /// for a 6-max blueprint; its history keys will not mean what they meant in training).
    Policy,
}

#[derive(Clone)]
pub struct PolicySpec {
    pub label: String,
    pub main: Arc<dyn PolicyFactory>,
    /// Embed short-handed hands in the 6-max game (blueprints) instead of a native n-player
    /// engine (heuristics).
    pub embed6: bool,
    pub fallback: Arc<dyn PolicyFactory>,
    pub hu: HuMode,
}

#[derive(Clone, Debug)]
pub struct SessionConfig {
    pub buy_in: i64,
    pub target_hands: usize,
    pub seed: u64,
    pub auto_rebuy: bool,
    pub raw: bool,
    pub sizing: String,
    pub buckets: String,
    /// Stack-depth limit in big blinds (the blueprint/search are trained at 100 bb; Pluribus resets
    /// stacks every hand). `0` disables both layers. Layer 1: after a hand, if our stack exceeds
    /// this many bb, leave the table and rejoin the lobby with `buy_in`. Layer 2: never commit more
    /// than this many bb in total in one hand.
    pub max_stack_bb: f64,
}

/// Hands to wait before trying a stack reset again after the server refused/ignored one.
const RESET_COOLDOWN_HANDS: u64 = 25;
/// Resend `join_lobby` if no table/lobby confirmation arrived within this long.
const REJOIN_RETRY: Duration = Duration::from_secs(15);
const REJOIN_MAX_TRIES: u32 = 6;

/// Stack-reset state machine (layer 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResetState {
    Idle,
    /// `leave_table` sent after a hand; waiting for our own `player_left`.
    Leaving { hands_seen: u32 },
    /// Left; `join_lobby` sent; waiting for `table_joined`.
    Rejoining { since: Instant, tries: u32 },
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Counters {
    pub hands_started: u64,
    pub hands_done: u64,
    pub decisions: u64,
    pub acks: u64,
    pub rejects: u64,
    pub retries: u64,
    pub fallbacks: u64,
    /// Decisions where the policy itself fell back (e.g. search timeout -> blueprint action).
    pub policy_fallbacks: u64,
    pub desync_hands: u64,
    pub issues: u64,
    pub anomalies: u64,
    pub resyncs: u64,
    pub rebuys: u64,
    /// Completed stack resets (left an over-deep table and rejoined with the normal buy-in).
    pub stack_resets: u64,
    /// Resets that the server refused/ignored (fell back to the effective-stack guard).
    pub reset_aborts: u64,
    /// Decisions where the effective-stack guard replaced the policy's action.
    pub stack_guards: u64,
}

#[derive(PartialEq, Debug)]
enum PendState {
    Sent,
    Echoed,
    Synth,
}

struct Pending {
    client_action_id: String,
    hand_id: String,
    turn_token: String,
    action: OpAction,
    state: PendState,
    retried: bool,
    seat: usize,
}

struct HandCtx {
    hand_id: String,
    hand_no: u64,
    hero_op: usize,
    dealer: usize,
    sb: i64,
    bb: i64,
    hole: Option<[Card; 2]>,
    board: Vec<Card>,
    events: Vec<ActionObs>,
    sim: Option<Sim>,
    start_stacks: BTreeMap<usize, i64>,
    names: BTreeMap<usize, String>,
    decisions: u32,
    fallbacks: u32,
    issues: u32,
    desync: bool,
    policy: String,
    hint_stacks: BTreeMap<usize, i64>,
    /// Seats that joined while this hand was running (not dealt in).
    joined_mid: BTreeSet<usize>,
    rebuilds: u32,
}

pub struct Session {
    pub cfg: SessionConfig,
    spec: PolicySpec,
    sink: Box<dyn TraceSink>,
    pub counters: Counters,
    hand: Option<HandCtx>,
    pending: Option<Pending>,
    table_id: Option<String>,
    roster: BTreeMap<usize, (Option<String>, Option<i64>)>,
    last_table_seq: u64,
    handled_tokens: HashSet<String>,
    hand_counter: u64,
    action_counter: u64,
    pub hero_name: Option<String>,
    pub agent_id: Option<String>,
    pub done: bool,
    pub fatal: Option<String>,
    awaiting_rebuy: bool,
    pub in_lobby_or_table: bool,
    last_valid: Option<ValidSet>,
    reset: ResetState,
    reset_cooldown_until: u64,
    my_seat: Option<usize>,
}

fn act_label(a: Action) -> String {
    match a {
        Action::Fold => "fold".into(),
        Action::Check => "check".into(),
        Action::Call => "call".into(),
        Action::RaiseTo(t) => format!("raise:{t}"),
    }
}

impl Session {
    pub fn new(cfg: SessionConfig, spec: PolicySpec, sink: Box<dyn TraceSink>) -> Session {
        let mut s = Session {
            cfg,
            spec,
            sink,
            counters: Counters::default(),
            hand: None,
            pending: None,
            table_id: None,
            roster: BTreeMap::new(),
            last_table_seq: 0,
            handled_tokens: HashSet::new(),
            hand_counter: 0,
            action_counter: 0,
            hero_name: None,
            agent_id: None,
            done: false,
            fatal: None,
            awaiting_rebuy: false,
            in_lobby_or_table: false,
            last_valid: None,
            reset: ResetState::Idle,
            reset_cooldown_until: 0,
            my_seat: None,
        };
        s.emit(json!({
            "kind": "session_start",
            "policy": s.spec.label,
            "embed6": s.spec.embed6,
            "hu_mode": format!("{:?}", s.spec.hu),
            "buy_in": s.cfg.buy_in,
            "target_hands": s.cfg.target_hands,
            "seed": s.cfg.seed,
            "max_stack_bb": s.cfg.max_stack_bb,
            "sizing": s.cfg.sizing,
            "buckets": s.cfg.buckets,
            "scale": crate::SCALE,
        }));
        s
    }

    fn emit(&mut self, rec: Value) {
        self.sink.emit(rec);
    }

    /// Trace a record from outside (IO layer); counts as an anomaly if it is one.
    pub fn note(&mut self, rec: Value) {
        if rec.get("kind").and_then(Value::as_str) == Some("anomaly") {
            self.counters.anomalies += 1;
        }
        self.emit(rec);
    }

    fn anomaly(&mut self, what: &str, details: Value) {
        self.counters.anomalies += 1;
        self.emit(json!({"kind": "anomaly", "what": what, "details": details}));
    }

    fn out(&mut self, v: Value) -> Value {
        if self.cfg.raw {
            self.emit(json!({"kind": "raw", "dir": "out", "msg": v}));
        }
        v
    }

    /// Messages to send right after (re)connecting with no known table.
    pub fn join_messages(&mut self) -> Vec<Value> {
        self.in_lobby_or_table = true;
        if matches!(self.reset, ResetState::Leaving { .. }) {
            self.reset = ResetState::Idle; // a reconnect / table_closed overtook the reset
        }
        let mut v = vec![self.out(proto::join_lobby(self.cfg.buy_in))];
        if self.cfg.auto_rebuy {
            v.push(self.out(proto::set_auto_rebuy(true)));
        }
        v
    }

    /// After a reconnect when the server says we are still seated.
    pub fn resync_message(&mut self, table_id: Option<&str>) -> Option<Value> {
        let t = table_id.map(str::to_string).or_else(|| self.table_id.clone())?;
        self.counters.resyncs += 1;
        let m = proto::resync_request(&t, self.last_table_seq);
        Some(self.out(m))
    }

    pub fn leave_messages(&mut self) -> Vec<Value> {
        vec![self.out(proto::leave_table())]
    }

    pub fn finish(&mut self, reason: &str) {
        let c = serde_json::to_value(&self.counters).unwrap_or(Value::Null);
        self.emit(json!({"kind": "session_end", "reason": reason, "counters": c}));
    }

    // ------------------------------------------------------------------ dispatch

    pub fn on_message(&mut self, msg: &Value) -> Vec<Value> {
        if self.cfg.raw {
            self.emit(json!({"kind": "raw", "dir": "in", "msg": msg}));
        }
        // Envelope: ignore duplicates/regressions of table_seq for table-scoped messages.
        if let Some(seq) = proto::get_u64(msg, "table_seq") {
            if seq != 0 && seq <= self.last_table_seq && proto::msg_type(msg) != "resync_response" {
                return Vec::new();
            }
            if seq > self.last_table_seq {
                self.last_table_seq = seq;
            }
        }
        let mut out = Vec::new();
        match proto::msg_type(msg) {
            "connected" => {
                self.hero_name = proto::get_str(msg, "name").map(str::to_string);
                self.agent_id = proto::get_str(msg, "agent_id").map(str::to_string);
            }
            "lobby_joined" | "auto_rebuy_set" | "auto_rebuy_scheduled" | "chips_skimmed" => {}
            "table_joined" => self.on_table_joined(msg),
            "player_joined" => {
                if let Some(seat) = proto::get_i64(msg, "seat") {
                    let name = proto::get_str(msg, "name").map(str::to_string);
                    let stack = proto::get_i64(msg, "stack");
                    self.roster.insert(seat as usize, (name, stack));
                    if let Some(h) = self.hand.as_mut() {
                        h.joined_mid.insert(seat as usize);
                    }
                }
            }
            "player_left" => out.extend(self.on_player_left(msg)),
            "hand_start" => self.on_hand_start(msg),
            "hole_cards" => {
                if let Some(h) = self.hand.as_mut() {
                    let c = proto::parse_cards_value(msg.get("cards"));
                    if c.len() == 2 {
                        h.hole = Some([c[0], c[1]]);
                    }
                }
            }
            "player_action" => self.on_player_action(msg),
            "community_cards" => self.on_community(msg),
            "your_turn" => out.extend(self.on_your_turn(msg)),
            "action_ack" => self.on_ack(msg),
            "hand_result" => self.on_hand_result(msg, &mut out),
            "table_state" => out.extend(self.on_table_state(msg)),
            "resync_response" => out.extend(self.on_resync(msg)),
            "busted" => {
                self.abandon_hand("busted");
                if !self.cfg.auto_rebuy && !self.done {
                    self.awaiting_rebuy = true;
                    self.counters.rebuys += 1;
                    let m = self.out(proto::rebuy());
                    out.push(m);
                }
            }
            "rebuy_confirmed" => {
                if self.awaiting_rebuy && !self.done {
                    self.awaiting_rebuy = false;
                    let m = self.out(proto::join_lobby(self.cfg.buy_in));
                    out.push(m);
                }
            }
            "table_closed" | "season_ended" => {
                self.abandon_hand(proto::msg_type(msg));
                if matches!(self.reset, ResetState::Leaving { .. }) {
                    self.reset = ResetState::Idle;
                }
                self.table_id = None;
                self.roster.clear();
                self.last_table_seq = 0;
                if !self.done {
                    out.extend(self.join_messages());
                }
            }
            "action_rejected" => out.extend(self.on_reject(msg)),
            "error" => out.extend(self.on_error(msg)),
            _ => {}
        }
        out
    }

    fn on_table_joined(&mut self, msg: &Value) {
        self.reset = ResetState::Idle;
        self.my_seat = proto::get_i64(msg, "seat").map(|x| x as usize);
        self.table_id = proto::get_str(msg, "table_id").map(str::to_string);
        self.last_table_seq = 0;
        self.roster.clear();
        for p in proto::parse_players(msg.get("players")) {
            self.roster.insert(p.seat, (p.name, p.stack));
        }
    }

    fn on_error(&mut self, msg: &Value) -> Vec<Value> {
        let code = proto::get_str(msg, "code").unwrap_or("");
        if proto::is_rejection_code(code) {
            return self.on_reject(msg);
        }
        match code {
            "auth_failed" => {
                self.fatal = Some("auth_failed: check the API key".into());
                self.done = true;
            }
            "insufficient_funds" => {
                self.anomaly("insufficient_funds", msg.clone());
                if !self.done {
                    let m = self.out(proto::rebuy());
                    self.awaiting_rebuy = true;
                    return vec![m];
                }
            }
            // After a bust the season balance may still hold chips: the server refuses the
            // rebuy and expects us to simply rejoin a table (seen live; otherwise we idle forever).
            "cannot_rebuy" => {
                self.anomaly("cannot_rebuy", msg.clone());
                self.awaiting_rebuy = false;
                if !self.done {
                    return self.join_messages();
                }
            }
            "not_at_table" | "no_hand_in_progress" => {
                if matches!(self.reset, ResetState::Leaving { .. }) {
                    // Already off the table: carry on with the rejoin.
                    return self.finish_leave("not_at_table");
                }
            }
            "already_seated" if !matches!(self.reset, ResetState::Idle) => {
                self.reset_failed(&format!("already_seated: {msg}"));
            }
            _ if matches!(self.reset, ResetState::Leaving { .. }) => {
                self.anomaly("server_error", msg.clone());
                self.reset_failed(&format!("leave_table refused: {msg}"));
            }
            _ => self.anomaly("server_error", msg.clone()),
        }
        Vec::new()
    }

    // ------------------------------------------------------------------ hand lifecycle

    fn on_hand_start(&mut self, msg: &Value) {
        let hand_id = proto::get_str(msg, "hand_id").unwrap_or("").to_string();
        if self.hand.as_ref().is_some_and(|h| h.hand_id == hand_id) {
            return; // replay of a known hand_start (resync)
        }
        self.abandon_hand("new hand_start");
        if let ResetState::Leaving { hands_seen } = self.reset {
            if hands_seen >= 2 {
                self.reset_failed("leave_table not honoured after 3 further hands");
            } else {
                self.reset = ResetState::Leaving { hands_seen: hands_seen + 1 };
            }
        }
        let hero_op = proto::get_i64(msg, "seat").unwrap_or(0) as usize;
        let dealer = proto::get_i64(msg, "dealer_seat").unwrap_or(0) as usize;
        let (sb, bb) = msg
            .get("blinds")
            .map(|b| (proto::get_i64(b, "small_blind").unwrap_or(10), proto::get_i64(b, "big_blind").unwrap_or(20)))
            .unwrap_or((10, 20));
        self.hand_counter += 1;
        self.counters.hands_started += 1;
        let hint_stacks: BTreeMap<usize, i64> = self.roster.iter().filter_map(|(s, (_, st))| st.map(|x| (*s, x))).collect();
        let names: BTreeMap<usize, String> = self.roster.iter().filter_map(|(s, (n, _))| n.clone().map(|x| (*s, x))).collect();
        self.hand = Some(HandCtx {
            hand_id,
            hand_no: self.hand_counter,
            hero_op,
            dealer,
            sb,
            bb,
            hole: None,
            board: Vec::new(),
            events: Vec::new(),
            sim: None,
            start_stacks: BTreeMap::new(),
            names,
            decisions: 0,
            fallbacks: 0,
            issues: 0,
            desync: false,
            policy: String::new(),
            hint_stacks,
            joined_mid: BTreeSet::new(),
            rebuilds: 0,
        });
        self.pending = None;
    }

    fn abandon_hand(&mut self, why: &str) {
        if let Some(h) = self.hand.take() {
            self.anomaly("hand_abandoned", json!({"hand_id": h.hand_id, "why": why, "events": h.events.len()}));
        }
        self.pending = None;
    }

    fn flush_hero_pending(&mut self) {
        let Some(p) = self.pending.as_mut() else { return };
        if p.state != PendState::Sent {
            return;
        }
        let Some(h) = self.hand.as_mut() else { return };
        if h.hand_id != p.hand_id {
            return;
        }
        let (kind, amount) = match p.action.action.as_str() {
            "fold" => (ObsKind::Fold, None),
            "check" => (ObsKind::Check, None),
            "call" => (ObsKind::Call, None),
            "raise" => (ObsKind::Raise, p.action.amount),
            _ => (ObsKind::AllIn, None),
        };
        let o = ActionObs {
            seat: p.seat,
            name: self.hero_name.clone(),
            kind,
            raw_action: p.action.action.clone(),
            amount,
            street: None,
            stack: None,
            stack_before: None,
            delta: None,
            pot: None,
            amount_mode: None,
        };
        p.state = PendState::Synth;
        self.record_obs(o, true);
    }

    /// Append to the hand's event list and apply to the engine if it exists.
    fn record_obs(&mut self, o: ActionObs, synthetic: bool) {
        let Some(h) = self.hand.as_mut() else { return };
        let mut fail = None;
        if let Some(sim) = h.sim.as_mut() {
            if !h.desync {
                if let Err(e) = sim.apply_obs(&o) {
                    fail = Some(e);
                }
                let iss = sim.take_issues();
                h.issues += iss.len() as u32;
                if !iss.is_empty() {
                    let hid = h.hand_id.clone();
                    self.counters.issues += iss.len() as u64;
                    self.sink.emit(json!({"kind": "anomaly", "what": "engine_server_mismatch", "details": {"hand_id": hid, "issues": iss, "synthetic": synthetic}}));
                    self.counters.anomalies += 1;
                }
            }
        }
        let Some(h) = self.hand.as_mut() else { return };
        if let Some(n) = &o.name {
            h.names.insert(o.seat, n.clone());
        }
        h.events.push(o);
        if let Some(e) = fail {
            // Drop the replay; the next decision rebuilds it from the full event list (this
            // usually means the set of dealt-in players was guessed wrong at the first build).
            h.sim = None;
            h.rebuilds += 1;
            let hid = h.hand_id.clone();
            self.counters.anomalies += 1;
            self.sink.emit(json!({"kind": "anomaly", "what": "replay_failed_will_rebuild", "details": {"hand_id": hid, "error": e}}));
        }
    }

    fn on_player_action(&mut self, msg: &Value) {
        let Some(o) = proto::parse_player_action(msg) else {
            self.anomaly("unparsable_player_action", msg.clone());
            return;
        };
        if self.hand.is_none() {
            return;
        }
        // Echo of our own action?
        let hero = self.hand.as_ref().map(|h| h.hero_op);
        if Some(o.seat) == hero {
            if let Some(p) = self.pending.as_mut() {
                match p.state {
                    PendState::Synth => {
                        // Already applied synthetically; drop the late echo.
                        p.state = PendState::Echoed;
                        return;
                    }
                    PendState::Sent => p.state = PendState::Echoed,
                    PendState::Echoed => {}
                }
            }
        } else {
            self.flush_hero_pending();
        }
        self.record_obs(o, false);
    }

    fn on_community(&mut self, msg: &Value) {
        self.flush_hero_pending();
        let cards = proto::parse_cards_value(msg.get("cards"));
        let Some(h) = self.hand.as_mut() else { return };
        match cards.len() {
            0 => {}
            1 => h.board.push(cards[0]),
            _ => h.board = cards,
        }
        let b = h.board.clone();
        if let Some(sim) = h.sim.as_mut() {
            sim.set_board(&b);
        }
    }

    fn on_ack(&mut self, msg: &Value) {
        self.counters.acks += 1;
        let status = proto::get_str(msg, "status").unwrap_or("");
        let id = proto::get_str(msg, "client_action_id").unwrap_or("").to_string();
        self.emit(json!({"kind": "ack", "client_action_id": id, "status": status}));
    }

    fn on_reject(&mut self, msg: &Value) -> Vec<Value> {
        self.counters.rejects += 1;
        self.emit(json!({"kind": "reject", "msg": msg}));
        let code = proto::get_str(msg, "code").unwrap_or("").to_string();
        let Some(p) = self.pending.as_mut() else { return Vec::new() };
        if p.state == PendState::Sent && !p.retried && code == "invalid_action" {
            // Retry once with the safest legal action on the same turn.
            p.retried = true;
            self.counters.retries += 1;
            let v = self.last_valid.clone().unwrap_or_default();
            let a = passive(&v);
            self.action_counter += 1;
            let id = format!("plu-{}-r{}", self.hand_counter, self.action_counter);
            let Some(p) = self.pending.as_mut() else { return Vec::new() };
            p.client_action_id = id.clone();
            p.action = a.clone();
            let m = proto::action_msg(&p.hand_id, &a.action, a.amount, &id, &p.turn_token);
            return vec![self.out(m)];
        }
        // Not retryable (stale token / not our turn / already retried): the server will
        // auto-act at the deadline; do not apply anything locally.
        if matches!(p.state, PendState::Sent) {
            self.pending = None;
        }
        Vec::new()
    }
}

// ---------------------------------------------------------------------- decisions

/// Fallback reason strings end up in the trace.
struct Chosen {
    op: OpAction,
    fidelity: Option<Fidelity>,
    nl_action: Option<Action>,
    dist: Vec<(Action, f64)>,
    u: f64,
    fallback: Option<String>,
    /// Policy-reported fallback (search timeout/panic -> blueprint action); not a session fallback.
    policy_note: Option<String>,
    issues: Vec<String>,
}

impl Session {
    fn on_your_turn(&mut self, msg: &Value) -> Vec<Value> {
        self.flush_hero_pending();
        let hand_id = proto::get_str(msg, "hand_id").unwrap_or("").to_string();
        let token = proto::get_str(msg, "turn_token").unwrap_or("").to_string();
        if !token.is_empty() && !self.handled_tokens.insert(token.clone()) {
            return Vec::new(); // duplicate delivery (e.g. via resync snapshot)
        }
        if self.handled_tokens.len() > 512 {
            self.handled_tokens.clear();
        }
        let va = proto::parse_valid_actions(msg.get("valid_actions"));
        let valid = ValidSet::from_actions(&va, proto::get_i64(msg, "min_raise"), proto::get_i64(msg, "max_raise"));
        self.last_valid = Some(valid.clone());
        let server_pot = proto::get_i64(msg, "pot");
        let board_msg = proto::parse_cards_value(msg.get("community_cards"));
        let players = proto::parse_players(msg.get("players"));
        let t0 = Instant::now();

        let mut ctx = match self.hand.take() {
            Some(h) if h.hand_id == hand_id || hand_id.is_empty() => Some(h),
            other => {
                self.hand = other;
                None
            }
        };
        let seed = self.cfg.seed;
        let mut chosen = Chosen {
            op: passive(&valid),
            fidelity: None,
            nl_action: None,
            dist: Vec::new(),
            u: 0.0,
            fallback: None,
            policy_note: None,
            issues: Vec::new(),
        };
        let mut meta = json!({});
        if let Some(h) = ctx.as_mut() {
            if board_msg.len() > h.board.len() {
                h.board = board_msg.clone();
            }
            if h.sim.is_none() && !h.desync {
                match Self::build_sim(h, &self.spec, &players, server_pot, mix(seed, h.hand_no)) {
                    Ok(notes) => {
                        if !notes.is_empty() {
                            self.sink.emit(json!({"kind":"note","what":"stack_derivation","details":{"hand_id":h.hand_id,"notes":notes}}));
                        }
                    }
                    Err(e) => {
                        h.desync = true;
                        self.counters.anomalies += 1;
                        self.sink.emit(json!({"kind":"anomaly","what":"desync","details":{"hand_id":h.hand_id,"error":e,"stage":"build_sim"}}));
                    }
                }
            }
            let board_now = h.board.clone();
            if let Some(sim) = h.sim.as_mut() {
                sim.set_board(&board_now);
            }
            if h.desync || h.sim.is_none() {
                chosen.fallback = Some("desync".into());
            } else {
                let sim = h.sim.as_mut().unwrap();
                let mut issues = sim.compare_legal(&valid, server_pot);
                for p in &players {
                    if let (Some(st), Some(mine)) = (p.stack, sim.stack_op(p.seat)) {
                        if st != mine {
                            issues.push(format!("stack seat {}: server {st} vs engine {mine}", p.seat));
                        }
                    }
                }
                issues.extend(sim.take_issues());
                chosen.issues = issues;
                let mut rng = Rng::new(mix(mix(seed, h.hand_no), h.decisions as u64 + 1));
                chosen.u = rng.f64();
                match sim.decide(chosen.u) {
                    Ok(d) => {
                        let (op, fid) = map_action(d.chosen, &valid);
                        chosen.dist = d.dist;
                        chosen.policy_note = d.note;
                        chosen.nl_action = Some(d.chosen);
                        chosen.op = op;
                        chosen.fidelity = Some(fid);
                    }
                    Err(e) => {
                        h.desync = true;
                        chosen.fallback = Some(format!("decide: {e}"));
                    }
                }
            }
            meta = json!({
                "hand_no": h.hand_no, "hero_seat": h.hero_op, "policy": h.policy,
                "position": h.sim.as_ref().map(|s| s.layout.position_name(h.hero_op)).unwrap_or("?"),
                "n_players": h.sim.as_ref().map(|s| s.layout.n_real),
                "hole": h.hole.map(|x| cards_to_strs(&x)), "board": cards_to_strs(&h.board),
                "street": h.sim.as_ref().map(|s| format!("{:?}", s.street())),
            });
            h.decisions += 1;
            h.issues += chosen.issues.len() as u32;
            if chosen.fallback.is_some() {
                h.fallbacks += 1;
            }
        } else {
            chosen.fallback = Some("no hand context".into());
            self.counters.anomalies += 1;
            self.sink.emit(json!({"kind":"anomaly","what":"your_turn_without_hand","details":{"hand_id":hand_id}}));
        }
        self.hand = ctx.or(self.hand.take());
        // Last line of defence: never send something the server did not offer.
        let mut stack_guard: Option<String> = None;
        if let (Some(h), true) = (self.hand.as_ref(), chosen.fallback.is_none()) {
            if let (Some(cap), Some(sim)) = (self.cap_chips(h.bb), h.sim.as_ref()) {
                let me = h.hero_op;
                if let (Some(total), Some(bet), Some(stack)) = (sim.contributed_op(me), sim.street_bet_op(me), sim.stack_op(me)) {
                    if let Some((op, note)) = cap_action(&chosen.op, &valid, cap, total - bet, total + stack) {
                        stack_guard = Some(note);
                        chosen.op = op;
                        self.counters.stack_guards += 1;
                    }
                }
            }
        }
        if !is_legal(&chosen.op, &valid) {
            self.counters.anomalies += 1;
            self.sink.emit(json!({"kind":"anomaly","what":"illegal_choice_replaced","details":{"chosen":chosen.op.label(),"valid":valid.describe()}}));
            chosen.op = passive(&valid);
            chosen.fallback.get_or_insert_with(|| "illegal mapped action".into());
        }
        if chosen.fallback.is_some() {
            self.counters.fallbacks += 1;
        }
        if chosen.policy_note.is_some() {
            self.counters.policy_fallbacks += 1;
        }
        self.counters.decisions += 1;
        self.counters.issues += chosen.issues.len() as u64;
        self.action_counter += 1;
        let cid = format!("plu-{}-{}", self.hand_counter, self.action_counter);
        let latency_us = t0.elapsed().as_micros() as u64;
        self.emit(json!({
            "kind": "decision", "hand_id": hand_id, "client_action_id": cid, "meta": meta,
            "pot": server_pot, "valid": valid.describe(),
            "players": players.iter().map(|p| json!({"seat": p.seat, "stack": p.stack, "in_hand": p.in_hand, "status": p.status})).collect::<Vec<_>>(),
            "issues": chosen.issues, "fallback": chosen.fallback, "fallback_reason": chosen.policy_note,
            "dist": chosen.dist.iter().map(|(a, p)| json!([act_label(*a), (p * 1e6).round() / 1e6])).collect::<Vec<_>>(),
            "u": chosen.u, "chosen": chosen.nl_action.map(act_label),
            "sent": chosen.op.label(), "stack_guard": stack_guard, "fidelity": chosen.fidelity.as_ref().map(|f| format!("{f:?}")),
            "latency_us": latency_us,
        }));
        let seat = self.hand.as_ref().map(|h| h.hero_op).unwrap_or(0);
        self.pending = Some(Pending {
            client_action_id: cid.clone(),
            hand_id: hand_id.clone(),
            turn_token: token.clone(),
            action: chosen.op.clone(),
            state: PendState::Sent,
            retried: false,
            seat,
        });
        let m = proto::action_msg(&hand_id, &chosen.op.action, chosen.op.amount, &cid, &token);
        vec![self.out(m)]
    }

    /// Build the engine replay at the hero's first decision. Tries the plausible sets of
    /// dealt-in players and keeps the first whose replay is consistent with the server's pot.
    /// Returns informational notes.
    fn build_sim(
        h: &mut HandCtx,
        spec: &PolicySpec,
        players: &[PlayerInfo],
        server_pot: Option<i64>,
        seed: u64,
    ) -> Result<Vec<String>, String> {
        let hole = h.hole.ok_or("your_turn before hole_cards")?;
        let evidence: BTreeSet<usize> = h
            .events
            .iter()
            .filter(|e| e.kind != ObsKind::Ignored)
            .map(|e| e.seat)
            .chain(std::iter::once(h.hero_op))
            .collect();
        for p in players {
            if let Some(n) = &p.name {
                h.names.insert(p.seat, n.clone());
            }
        }
        // Dealt-in set: evidence (seats that acted, hero) plus any subset of the other seated
        // players (your_turn.players carries no status and seats can join mid-hand). Try the
        // largest subsets first; a candidate is valid when every event replays, the hero is to
        // act and the pot matches the server's.
        let optional: Vec<usize> = players
            .iter()
            .filter(|p| p.dealt_in() != Some(false) && !h.joined_mid.contains(&p.seat) && !evidence.contains(&p.seat))
            .map(|p| p.seat)
            .collect();
        let mut masks: Vec<u32> = if optional.len() <= 5 { (0..(1u32 << optional.len())).collect() } else { vec![(1u32 << optional.len()) - 1, 0] };
        masks.sort_by_key(|m| std::cmp::Reverse(m.count_ones()));
        let mut best: Option<(Sim, BTreeMap<usize, i64>, String, Vec<String>)> = None;
        let mut first_err = None;
        for mask in masks {
            let mut parts = evidence.clone();
            for (i, s) in optional.iter().enumerate() {
                if mask >> i & 1 == 1 {
                    parts.insert(*s);
                }
            }
            if parts.len() < 2 {
                continue;
            }
            match Self::try_build(h, spec, &parts, players, hole, seed) {
                Ok((sim, stacks, label, notes)) => {
                    let valid = sim.hero_to_act() && server_pot.map_or(true, |p| p == sim.pot_op());
                    if best.is_none() || valid {
                        best = Some((sim, stacks, label, notes));
                    }
                    if valid {
                        break;
                    }
                }
                Err(e) => {
                    first_err.get_or_insert(e);
                }
            }
        }
        let (mut sim, stacks, label, mut notes) = best.ok_or_else(|| first_err.unwrap_or_else(|| "no layout".into()))?;
        notes.extend(sim.take_issues());
        sim.set_board(&h.board);
        h.start_stacks = stacks;
        h.policy = label;
        h.sim = Some(sim);
        Ok(notes)
    }

    fn try_build(
        h: &HandCtx,
        spec: &PolicySpec,
        parts: &BTreeSet<usize>,
        players: &[PlayerInfo],
        hole: [Card; 2],
        seed: u64,
    ) -> Result<(Sim, BTreeMap<usize, i64>, String, Vec<String>), String> {
        let n = parts.len();
        let fallback = n == 2 && spec.hu == HuMode::Fallback;
        let (factory, embed6, label): (Arc<dyn PolicyFactory>, bool, String) = if fallback {
            (spec.fallback.clone(), false, format!("fallback:{}", spec.fallback.name()))
        } else {
            (spec.main.clone(), spec.embed6, spec.label.clone())
        };
        const BIG: i64 = 200_000;
        let mut notes = Vec::new();
        let reported: BTreeMap<usize, i64> = players.iter().filter_map(|p| p.stack.map(|s| (p.seat, s))).collect();
        let order = engine_order(parts, h.dealer);
        let blind_of = |s: usize| -> i64 {
            if order.first() == Some(&s) {
                h.sb
            } else if order.get(1) == Some(&s) {
                h.bb
            } else {
                0
            }
        };
        // First recorded action of each seat: its stack_before (or stack_after + delta).
        let mut first: BTreeMap<usize, Option<i64>> = BTreeMap::new();
        for e in h.events.iter().filter(|e| e.kind != ObsKind::Ignored) {
            first
                .entry(e.seat)
                .or_insert_with(|| e.stack_before.or_else(|| e.stack.zip(e.delta).map(|(s, d)| s + d)));
        }
        let mut stacks: BTreeMap<usize, i64> = BTreeMap::new();
        let direct = parts.iter().all(|s| match first.get(s) {
            Some(Some(_)) => true,
            Some(None) => false,
            None => reported.contains_key(s) || h.hint_stacks.contains_key(s),
        });
        if direct {
            // Exact: stack before the first action (or the current stack of a seat that has
            // not acted yet) plus the blind that seat posted.
            for &s in parts {
                let base = match first.get(&s) {
                    Some(Some(b)) => *b,
                    _ => reported.get(&s).or_else(|| h.hint_stacks.get(&s)).copied().unwrap_or(BIG),
                };
                stacks.insert(s, base + blind_of(s));
            }
        } else {
            // Fallback (messages without stack_before): start = current stack + chips committed,
            // iterated because all-in caps depend on the stacks themselves.
            notes.push("start stacks derived from current stacks + replayed contributions".into());
            let contrib = |st: &BTreeMap<usize, i64>| -> Result<BTreeMap<usize, i64>, String> {
                let layout = Layout::build(parts, h.dealer, st, h.sb, h.bb, embed6)?;
                let mut sim = Sim::new(layout, h.hero_op, hole, None, 0)?;
                for e in &h.events {
                    sim.apply_obs(e)?;
                }
                Ok(parts.iter().map(|&s| (s, sim.contributed_op(s).unwrap_or(0))).collect())
            };
            stacks = parts.iter().map(|&s| (s, h.hint_stacks.get(&s).copied().unwrap_or(BIG))).collect();
            let mut c = contrib(&stacks)?;
            for _ in 0..4 {
                let next: BTreeMap<usize, i64> = parts
                    .iter()
                    .map(|&s| (s, reported.get(&s).map_or_else(|| h.hint_stacks.get(&s).copied().unwrap_or(BIG), |r| r + c[&s])))
                    .collect();
                if next == stacks {
                    break;
                }
                stacks = next;
                c = contrib(&stacks)?;
            }
        }
        for (s, st) in &stacks {
            if let Some(hint) = h.hint_stacks.get(s) {
                if hint != st {
                    notes.push(format!("start stack seat {s}: hint {hint} vs derived {st}"));
                }
            }
        }
        let layout = Layout::build(parts, h.dealer, &stacks, h.sb, h.bb, embed6)?;
        let mut sim = Sim::new(layout, h.hero_op, hole, Some(&*factory), seed)?;
        for e in &h.events {
            sim.apply_obs(e)?;
        }
        Ok((sim, stacks, label, notes))
    }

    // ------------------------------------------------------------------ results

    fn on_hand_result(&mut self, msg: &Value, out: &mut Vec<Value>) {
        self.flush_hero_pending();
        let Some(mut h) = self.hand.take() else {
            // A hand we never saw start (e.g. joined mid-hand). Still keep the roster fresh.
            for (seat, st) in proto::parse_seat_numbers(msg.get("final_stacks"), &["stack", "final_stack"]) {
                self.roster.entry(seat).or_insert((None, None)).1 = Some(st);
            }
            return;
        };
        self.pending = None;
        let final_stacks = proto::parse_seat_numbers(msg.get("final_stacks"), &["stack", "final_stack", "final"]);
        let winners = msg.get("winners").and_then(Value::as_array).cloned().unwrap_or_default();
        let shown = shown_from_value(msg.get("shown_cards"));
        let board_msg = proto::parse_cards_value(msg.get("community_cards").or_else(|| msg.get("board")));
        if board_msg.len() > h.board.len() {
            h.board = board_msg;
        }
        let hero = h.hero_op;
        let start_hero = h.start_stacks.get(&hero).or_else(|| h.hint_stacks.get(&hero)).copied();
        let end_hero = final_stacks.iter().find(|x| x.0 == hero).map(|x| x.1);
        let by_stacks = match (start_hero, end_hero) {
            (Some(s), Some(e)) => Some(e - s),
            _ => None,
        };
        let contrib_hero = h.sim.as_ref().and_then(|s| s.contributed_op(hero));
        // `payouts[].amount` is the gross amount paid to a seat (`winners[].amount` is net).
        let payouts = proto::parse_seat_numbers(msg.get("payouts"), &["amount"]);
        let won_gross: i64 = payouts.iter().filter(|x| x.0 == hero).map(|x| x.1).sum();
        let by_winners = if payouts.is_empty() { None } else { contrib_hero.map(|c| won_gross - c) };
        let (profit, src) = match (by_stacks, by_winners) {
            (Some(a), Some(b)) => {
                // They legitimately differ when an uncalled bet is refunded (the refund is not a
                // payout); `final_stacks` is authoritative. Keep the difference for debugging.
                if a != b {
                    self.sink.emit(json!({"kind":"note","what":"profit_sources_differ","details":{"hand_id":h.hand_id,"final_stacks":a,"payouts_minus_contrib":b}}));
                }
                (Some(a), "final_stacks")
            }
            (Some(a), None) => (Some(a), "final_stacks"),
            (None, Some(b)) => (Some(b), "winners-contrib"),
            (None, None) => (None, "unknown"),
        };
        for (seat, st) in &final_stacks {
            let e = self.roster.entry(*seat).or_insert((None, None));
            e.1 = Some(*st);
        }
        // Players table.
        let mut players = Vec::new();
        if let Some(sim) = &h.sim {
            for (&op, &nl) in &sim.layout.nl_of_op {
                players.push(HandPlayer {
                    seat: op,
                    name: h.names.get(&op).cloned(),
                    start_stack: h.start_stacks.get(&op).copied(),
                    contrib: sim.contributed_op(op),
                    folded: Some(sim.state.is_folded(nl)),
                    shown: shown.get(&op).cloned(),
                });
            }
        } else {
            let seats: BTreeSet<usize> = h.events.iter().map(|e| e.seat).chain(std::iter::once(hero)).collect();
            for op in seats {
                players.push(HandPlayer {
                    seat: op,
                    name: h.names.get(&op).cloned(),
                    start_stack: h.hint_stacks.get(&op).copied(),
                    contrib: None,
                    folded: None,
                    shown: shown.get(&op).cloned(),
                });
            }
        }
        let rec = HandRecord {
            hand_id: h.hand_id.clone(),
            table_id: self.table_id.clone(),
            hand_no: h.hand_no,
            hero_seat: hero,
            dealer_seat: h.dealer,
            sb: h.sb,
            bb: h.bb,
            n_players: h.sim.as_ref().map(|s| s.layout.n_real).unwrap_or(players.len()),
            position: h.sim.as_ref().map(|s| s.layout.position_name(hero).to_string()).unwrap_or_default(),
            hero_hole: h.hole.map(|x| cards_to_strs(&x)).unwrap_or_default(),
            board: cards_to_strs(&h.board),
            players,
            actions: h
                .events
                .iter()
                .filter(|e| e.kind != ObsKind::Ignored)
                .map(|e| HandAct { seat: e.seat, street: e.street.clone(), action: e.raw_action.clone(), amount: e.amount })
                .collect(),
            profit,
            profit_src: src.into(),
            lock_len: h.sim.as_ref().and_then(|s| s.lock_len),
            showdown: !shown.is_empty() || h.sim.as_ref().is_some_and(|s| s.state.showdown()),
            policy: h.policy.clone(),
            decisions: h.decisions,
            fallbacks: h.fallbacks,
            issues: h.issues,
            desync: h.desync,
        };
        self.counters.hands_done += 1;
        self.counters.desync_hands += h.desync as u64;
        let ts = now_ms();
        self.emit(json!({"kind": "hand", "t": ts, "hand": rec, "hero_end_stack": end_hero, "winners": winners}));
        if self.counters.hands_done as usize >= self.cfg.target_hands && !self.done {
            self.done = true;
            out.extend(self.leave_messages());
        } else if !self.done {
            let end = end_hero.or_else(|| self.roster.get(&hero).and_then(|r| r.1));
            if let Some(m) = self.maybe_reset(end, h.bb) {
                out.push(m);
            }
        }
    }

    // ------------------------------------------------------------------ stack reset (layer 1)

    fn cap_chips(&self, bb: i64) -> Option<i64> {
        (self.cfg.max_stack_bb > 0.0).then(|| (self.cfg.max_stack_bb * bb as f64).round() as i64)
    }

    /// Called right after a hand is closed (never mid-hand): leave if our stack is too deep.
    fn maybe_reset(&mut self, end_stack: Option<i64>, bb: i64) -> Option<Value> {
        let cap = self.cap_chips(bb)?;
        let st = end_stack?;
        if st <= cap || self.reset != ResetState::Idle || self.counters.hands_done < self.reset_cooldown_until {
            return None;
        }
        self.reset = ResetState::Leaving { hands_seen: 0 };
        self.emit(json!({"kind":"note","what":"stack_reset_leave","details":{"stack":st,"bb":bb,"cap":cap,"stack_bb":st as f64 / bb.max(1) as f64,"buy_in":self.cfg.buy_in}}));
        Some(self.out(proto::leave_table()))
    }

    fn reset_failed(&mut self, why: &str) {
        self.reset = ResetState::Idle;
        self.reset_cooldown_until = self.counters.hands_done + RESET_COOLDOWN_HANDS;
        self.counters.reset_aborts += 1;
        self.emit(json!({"kind":"note","what":"stack_reset_aborted","details":{"why":why,"cooldown_hands":RESET_COOLDOWN_HANDS,"fallback":"stack_guard"}}));
    }

    /// We are off the table: forget it and queue for a new one with the normal buy-in.
    fn finish_leave(&mut self, how: &str) -> Vec<Value> {
        self.counters.stack_resets += 1;
        self.table_id = None;
        self.roster.clear();
        self.last_table_seq = 0;
        self.in_lobby_or_table = true;
        self.reset = ResetState::Rejoining { since: Instant::now(), tries: 1 };
        self.emit(json!({"kind":"note","what":"stack_reset_rejoin","details":{"how":how,"buy_in":self.cfg.buy_in}}));
        vec![self.out(proto::join_lobby(self.cfg.buy_in))]
    }

    fn on_player_left(&mut self, msg: &Value) -> Vec<Value> {
        let Some(seat) = proto::get_i64(msg, "seat").map(|x| x as usize) else { return Vec::new() };
        let name = proto::get_str(msg, "name");
        let mine = match (name, self.hero_name.as_deref()) {
            (Some(n), Some(h)) => n == h,
            _ => self.my_seat == Some(seat),
        };
        self.roster.remove(&seat);
        if !mine || self.done {
            return Vec::new();
        }
        let reason = proto::get_str(msg, "reason").unwrap_or("");
        match self.reset {
            ResetState::Leaving { .. } if reason != "busted" => self.finish_leave("player_left"),
            ResetState::Idle if reason == "left" => {
                // We left (e.g. a leave that was queued after we gave up waiting): do not idle.
                self.anomaly("unexpected_self_left", msg.clone());
                self.finish_leave("late_player_left")
            }
            _ => Vec::new(),
        }
    }

    /// Periodic tick from the IO layer (read timeouts): retry a stalled rejoin.
    pub fn on_idle(&mut self) -> Vec<Value> {
        self.on_idle_at(Instant::now())
    }

    pub fn on_idle_at(&mut self, now: Instant) -> Vec<Value> {
        let ResetState::Rejoining { since, tries } = self.reset else { return Vec::new() };
        if self.done || now.saturating_duration_since(since) < REJOIN_RETRY {
            return Vec::new();
        }
        if tries >= REJOIN_MAX_TRIES {
            self.reset = ResetState::Idle;
            self.anomaly("rejoin_gave_up", json!({"tries": tries}));
            return Vec::new();
        }
        self.reset = ResetState::Rejoining { since: now, tries: tries + 1 };
        self.emit(json!({"kind":"note","what":"stack_reset_rejoin_retry","details":{"try":tries + 1}}));
        vec![self.out(proto::join_lobby(self.cfg.buy_in))]
    }

    fn on_table_state(&mut self, msg: &Value) -> Vec<Value> {
        if let Some(t) = proto::get_str(msg, "table_id") {
            self.table_id = Some(t.to_string());
        }
        let seats = proto::parse_players(msg.get("seats"));
        let fresh = self.hand.as_ref().map_or(true, |h| h.events.is_empty() && h.sim.is_none());
        if fresh {
            for p in &seats {
                if let Some(st) = p.stack {
                    let start = st + p.bet.unwrap_or(0);
                    let e = self.roster.entry(p.seat).or_insert((None, None));
                    e.1 = Some(start);
                    if p.name.is_some() {
                        e.0 = p.name.clone();
                    }
                    if let Some(h) = self.hand.as_mut() {
                        h.hint_stacks.insert(p.seat, start);
                    }
                }
            }
        }
        // Resync with a live turn for us: rebuild the private `your_turn`.
        let hero = msg.get("hero");
        if let Some(hero) = hero {
            if let Some(h) = self.hand.as_mut() {
                let c = proto::parse_cards_value(hero.get("hole_cards"));
                if c.len() == 2 && h.hole.is_none() {
                    h.hole = Some([c[0], c[1]]);
                }
            }
            let token = proto::get_str(hero, "turn_token");
            let actor = proto::get_i64(msg, "actor_seat");
            let seat = proto::get_i64(hero, "seat");
            if let (Some(token), Some(actor), Some(seat)) = (token, actor, seat) {
                if actor == seat && !self.handled_tokens.contains(token) {
                    let hand_id = proto::get_str(msg, "hand_id")
                        .map(str::to_string)
                        .or_else(|| self.hand.as_ref().map(|h| h.hand_id.clone()))
                        .unwrap_or_default();
                    let synth = json!({
                        "type": "your_turn", "hand_id": hand_id, "turn_token": token,
                        "valid_actions": hero.get("valid_actions").cloned().unwrap_or(json!([])),
                        "pot": msg.get("pot"), "community_cards": msg.get("board").cloned().unwrap_or(json!([])),
                        "players": msg.get("seats").cloned().unwrap_or(json!([])),
                        "min_raise": msg.get("min_raise_to"), "max_raise": msg.get("max_raise_to"),
                    });
                    self.emit(json!({"kind": "anomaly", "what": "your_turn_from_snapshot", "details": {"hand_id": hand_id}}));
                    return self.on_your_turn(&synth);
                }
            }
        }
        Vec::new()
    }

    fn on_resync(&mut self, msg: &Value) -> Vec<Value> {
        let mut out = Vec::new();
        if let Some(events) = msg.get("replayed_events").and_then(Value::as_array) {
            for ev in events.clone() {
                // Events are server messages (possibly wrapped as {"event": {...}}).
                let inner = ev.get("event").cloned().unwrap_or(ev);
                out.extend(self.on_message(&inner));
            }
        }
        if let Some(to) = proto::get_u64(msg, "to_table_seq") {
            self.last_table_seq = self.last_table_seq.max(to);
        }
        if let Some(snap) = msg.get("snapshot") {
            out.extend(self.on_table_state(snap));
        }
        out
    }
}
