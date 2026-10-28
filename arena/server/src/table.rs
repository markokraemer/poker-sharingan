//! Tables: shared state (readable by REST/WS handlers) and the per-table task that deals hands.
//!
//! Each table is one tokio task that owns the running `HandRunner`. Seat controllers are either
//! house bots (the bench Policy API, answered inline), remote players over WebSocket (answers
//! arrive as `TableCmd::Act`), or remote players with a webhook. A decision that is not answered
//! before its deadline is played as check-if-free-else-fold with `timeout: true`.

use crate::deck;
use crate::error::{ApiError, Res};
use crate::hand::*;
use crate::hub::{ConnTx, Hub};
use crate::matches::{self, HandPlan, MatchPoll, MatchRuntime};
use crate::model::*;
use crate::state::AppState;
use crate::util::*;
use bench::mix;
use bench::policy::{Player, View};
use nlhe::Action;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::Notify;
use tokio::time::{sleep, timeout_at, Instant};

#[derive(Clone, Debug)]
pub struct SeatState {
    pub player_id: Option<String>,
    pub name: Option<String>,
    pub kind: String,
    pub house: bool,
    pub stack: i64,
    pub stand_up: bool,
}

impl SeatState {
    pub fn empty() -> Self {
        SeatState { player_id: None, name: None, kind: "bot".into(), house: false, stack: 0, stand_up: false }
    }
}

pub struct TableInner {
    pub config: TableConfig,
    pub status: String,
    pub seats: Vec<SeatState>,
    pub button_seat: Option<usize>,
    pub hand_id: Option<String>,
    pub table_seq: u64,
    pub hand: Option<Value>,
    pub holes: HashMap<usize, [String; 2]>,
    pub match_id: Option<String>,
    pub owner: String,
    pub created_at: String,
    pub in_hand: bool,
    /// Messages delivered so far per recipient: `table_seq` is gap-free per recipient.
    pub rseq: HashMap<String, u64>,
}

pub enum TableCmd {
    Act { player_id: String, data: Value, conn: ConnTx },
    Resume { player_id: String, conn: ConnTx },
    Leave { player_id: String },
    Close,
}

pub struct TableShared {
    pub id: String,
    pub inner: Mutex<TableInner>,
    pub tx: UnboundedSender<TableCmd>,
    pub wake: Notify,
}

impl TableShared {
    pub fn new(id: String, owner: String, config: TableConfig, match_id: Option<String>) -> (Arc<Self>, UnboundedReceiver<TableCmd>) {
        let (tx, rx) = unbounded_channel();
        let seats = vec![SeatState::empty(); config.max_seats as usize];
        let inner = TableInner {
            config,
            status: "waiting".into(),
            seats,
            button_seat: None,
            hand_id: None,
            table_seq: 0,
            hand: None,
            holes: HashMap::new(),
            match_id,
            owner,
            created_at: now_iso(),
            in_hand: false,
            rseq: HashMap::new(),
        };
        (Arc::new(TableShared { id, inner: Mutex::new(inner), tx, wake: Notify::new() }), rx)
    }

    pub fn seat_json(inner: &TableInner, seat: usize, hub: &Hub) -> Value {
        let s = &inner.seats[seat];
        let status = match &s.player_id {
            None => "empty",
            Some(_) if s.stand_up => "sitting_out",
            Some(p) if !s.house && !hub.connected(p) => "disconnected",
            Some(_) => "seated",
        };
        json!({"seat": seat, "player_id": s.player_id, "name": s.name, "stack": s.stack, "status": status,
               "kind": s.kind, "house": s.house})
    }

    fn table_value(inner: &TableInner, id: &str, hub: &Hub) -> Value {
        json!({
            "table_id": id, "config": inner.config, "status": inner.status,
            "seats": (0..inner.seats.len()).map(|i| Self::seat_json(inner, i, hub)).collect::<Vec<_>>(),
            "button_seat": inner.button_seat, "hand_id": inner.hand_id, "table_seq": inner.table_seq,
            "match_id": inner.match_id, "owner": inner.owner, "created_at": inner.created_at,
        })
    }

    /// `Table` plus the current public `HandState` (no hole cards).
    pub fn table_json(&self, hub: &Hub) -> Value {
        let inner = self.inner.lock().unwrap();
        let mut v = Self::table_value(&inner, &self.id, hub);
        v["hand"] = inner.hand.clone().unwrap_or(Value::Null);
        v
    }

    /// Full snapshot message for one player (their hole cards included).
    pub fn snapshot_msg(&self, hub: &Hub, player_id: &str) -> String {
        let inner = self.inner.lock().unwrap();
        let mut d = Self::table_value(&inner, &self.id, hub);
        d["hand"] = inner.hand.clone().unwrap_or(Value::Null);
        let mut mine = serde_json::Map::new();
        for (seat, s) in inner.seats.iter().enumerate() {
            if s.player_id.as_deref() == Some(player_id) {
                if let Some(c) = inner.holes.get(&seat) {
                    mine.insert(seat.to_string(), json!(c));
                }
            }
        }
        d["hole_cards"] = Value::Object(mine);
        let mine_seq = inner.rseq.get(player_id).copied().unwrap_or(0);
        d["table_seq"] = json!(mine_seq);
        let mut env = json!({"v": 1, "type": "table_snapshot", "table_id": self.id, "table_seq": mine_seq,
                             "t_ms": now_ms(), "data": d});
        if let Some(h) = &inner.hand_id {
            env["hand_id"] = json!(h);
        }
        env.to_string()
    }

    /// Deliver a table event. Every recipient gets its own gap-free `table_seq` (events private to
    /// another seat do not consume its numbers). The returned envelope carries the table-wide
    /// counter and is what the hand event log stores.
    pub fn emit(&self, hub: &Hub, hand_id: Option<&str>, kind: &str, seq: Option<u32>, to: Option<usize>, data: Value) -> Value {
        let mut inner = self.inner.lock().unwrap();
        inner.table_seq += 1;
        let mut env = json!({"v": 1, "type": kind, "table_id": self.id, "table_seq": inner.table_seq,
                             "t_ms": now_ms(), "data": data});
        if let Some(h) = hand_id {
            env["hand_id"] = json!(h);
        }
        if let Some(s) = seq {
            env["seq"] = json!(s);
        }
        let mut rec: Vec<String> = match to {
            Some(seat) => inner.seats.get(seat).filter(|s| !s.house).and_then(|s| s.player_id.clone()).into_iter().collect(),
            None => inner.seats.iter().filter(|s| !s.house).filter_map(|s| s.player_id.clone()).collect(),
        };
        rec.sort();
        rec.dedup();
        let mut out = Vec::with_capacity(rec.len());
        for p in rec {
            let c = inner.rseq.entry(p.clone()).or_insert(0);
            *c += 1;
            env["table_seq"] = json!(*c);
            out.push((p, env.to_string()));
        }
        // Private events log (and resend) the recipient's own number; broadcasts the table-wide one.
        if to.is_none() || out.len() != 1 {
            env["table_seq"] = json!(inner.table_seq);
        }
        drop(inner);
        for (p, s) in out {
            hub.send(&p, &s);
        }
        env
    }

    pub fn emit_seat_update(&self, hub: &Hub, seat: usize) {
        let d = {
            let inner = self.inner.lock().unwrap();
            Self::seat_json(&inner, seat, hub)
        };
        self.emit(hub, None, "seat_update", None, None, d);
    }
}

/// Create and register a table and spawn its task.
pub fn spawn_table(app: &Arc<AppState>, id: String, owner: String, config: TableConfig, plan: Option<Arc<MatchRuntime>>, lane: usize) -> Arc<TableShared> {
    let match_id = plan.as_ref().map(|p| p.id.clone());
    let seed = config.seed.clone().unwrap_or_else(random_hex);
    let (shared, rx) = TableShared::new(id.clone(), owner, config, match_id);
    app.tables.write().unwrap().insert(id, shared.clone());
    let task = TableTask {
        app: app.clone(),
        shared: shared.clone(),
        rx,
        plan,
        lane,
        hand_counter: 0,
        banks: HashMap::new(),
        answered: HashSet::new(),
        seed,
        players: HashMap::new(),
        closed: false,
    };
    tokio::spawn(task.run());
    shared
}

struct TableTask {
    app: Arc<AppState>,
    shared: Arc<TableShared>,
    rx: UnboundedReceiver<TableCmd>,
    plan: Option<Arc<MatchRuntime>>,
    lane: usize,
    hand_counter: u32,
    banks: HashMap<String, u64>,
    answered: HashSet<String>,
    seed: String,
    players: HashMap<String, PlayerRecord>,
    closed: bool,
}

enum Decision {
    Act(Action, u64),
    Timeout(u64),
}

pub(crate) fn engine_legal(st: &nlhe::State, a: Action) -> bool {
    let l = st.legal();
    match a {
        Action::Fold => l.can_fold,
        Action::Check => l.can_check,
        Action::Call => !l.can_check,
        Action::RaiseTo(t) => l.can_raise && t >= l.min_raise_to && t <= l.max_raise_to,
    }
}

impl TableTask {
    fn hub(&self) -> &Hub {
        &self.app.hub
    }

    async fn run(mut self) {
        loop {
            if self.closed {
                break;
            }
            let Some(plan) = self.next_plan().await else { break };
            if let Err(e) = self.play_hand(plan).await {
                tracing::error!("table {}: hand failed: {}", self.shared.id, e);
                sleep(Duration::from_millis(500)).await;
            }
        }
        let mut inner = self.shared.inner.lock().unwrap();
        inner.status = "closed".into();
        inner.hand = None;
        drop(inner);
    }

    async fn next_plan(&mut self) -> Option<HandPlan> {
        loop {
            if self.closed {
                return None;
            }
            if let Some(rt) = self.plan.clone() {
                match rt.poll(self.lane) {
                    MatchPoll::Play(p) => return Some(p),
                    MatchPoll::Finished(status) => {
                        self.closed = true;
                        matches::lane_finished(self.app.clone(), rt, status).await;
                        return None;
                    }
                    MatchPoll::Wait => {}
                }
            } else if let Some(p) = self.free_plan() {
                return Some(p);
            }
            tokio::select! {
                _ = self.shared.wake.notified() => {}
                cmd = self.rx.recv() => match cmd {
                    Some(c) => self.handle_idle(c),
                    None => return None,
                },
                _ = sleep(Duration::from_millis(250)) => {}
            }
        }
    }

    fn handle_idle(&mut self, cmd: TableCmd) {
        match cmd {
            TableCmd::Act { conn, data, .. } => {
                self.send_error(&conn, ApiError::conflict("not_your_turn", "no decision is pending"), data["decision_id"].as_str());
            }
            TableCmd::Resume { .. } => {}
            TableCmd::Leave { player_id } => self.leave(&player_id),
            TableCmd::Close => self.closed = true,
        }
    }

    fn leave(&mut self, player_id: &str) {
        let mut changed = Vec::new();
        {
            let mut inner = self.shared.inner.lock().unwrap();
            let in_hand = inner.in_hand;
            for (i, s) in inner.seats.iter_mut().enumerate() {
                if s.player_id.as_deref() == Some(player_id) {
                    if in_hand {
                        s.stand_up = true;
                    } else {
                        *s = SeatState::empty();
                    }
                    changed.push(i);
                }
            }
        }
        for i in changed {
            self.shared.emit_seat_update(self.hub(), i);
        }
    }

    fn send_error(&self, conn: &ConnTx, e: ApiError, decision_id: Option<&str>) {
        let mut d = json!({"code": e.code, "message": e.message, "details": e.details});
        if let Some(id) = decision_id {
            d["decision_id"] = json!(id);
        }
        let env = json!({"v": 1, "type": "error", "table_id": self.shared.id, "t_ms": now_ms(), "data": d});
        let _ = conn.send(env.to_string());
    }

    /// Seats that would be dealt in the next free-table hand.
    fn free_plan(&mut self) -> Option<HandPlan> {
        let inner = self.shared.inner.lock().unwrap();
        let cfg = &inner.config;
        let eligible: Vec<usize> = inner
            .seats
            .iter()
            .enumerate()
            .filter(|(_, s)| match &s.player_id {
                Some(p) => !s.stand_up && (s.house || self.app.hub.connected(p)),
                None => false,
            })
            .map(|(i, _)| i)
            .collect();
        if eligible.len() < cfg.min_players_to_deal as usize || !eligible.iter().any(|&i| !inner.seats[i].house) {
            return None;
        }
        let forced = cfg.sandbox.as_ref().and_then(|s| s.button_seat).filter(|b| eligible.contains(b));
        let max = cfg.max_seats as usize;
        let button = forced.unwrap_or_else(|| match inner.button_seat {
            Some(prev) => (1..=max).map(|k| (prev + k) % max).find(|s| eligible.contains(s)).unwrap(),
            None => eligible[0],
        });
        let hand_no = self.hand_counter + 1;
        Some(HandPlan {
            hand_no,
            deal_no: None,
            rotation: None,
            button,
            assignment: None,
            dealt: eligible,
            deck_index: self.hand_counter as u64,
            crn_key: hand_no as u64,
            seed: self.seed.clone(),
            match_id: None,
            hide_cards: false,
        })
    }

    fn player(&mut self, pid: &str) -> Res<PlayerRecord> {
        if let Some(p) = self.players.get(pid) {
            return Ok(p.clone());
        }
        let p = self.app.store.get_player(pid)?.ok_or_else(|| ApiError::not_found("player"))?;
        self.players.insert(pid.to_string(), p.clone());
        Ok(p)
    }

    fn emit_evs(&self, hand_id: &str, evs: Vec<HandEvent>, log: &mut Vec<Value>, hide: bool) -> Vec<Value> {
        let mut out = Vec::new();
        for mut e in evs {
            if hide && e.kind == "hand_end" {
                redact_hand_end(&mut e.data);
            }
            let env = self.shared.emit(self.hub(), Some(hand_id), e.kind, e.seq, e.to, e.data);
            log.push(env.clone());
            out.push(env);
        }
        out
    }

    fn publish_view(&self, r: &HandRunner) {
        let mut inner = self.shared.inner.lock().unwrap();
        inner.hand = Some(r.public_state());
    }

    async fn play_hand(&mut self, plan: HandPlan) -> Res<()> {
        let started_ms = now_ms();
        let cfg = self.shared.inner.lock().unwrap().config.clone();
        let lean = cfg.is_benchmark();
        if let Some(assign) = &plan.assignment {
            for (s, pid) in assign.iter().enumerate() {
                let rec = self.player(pid)?;
                let mut inner = self.shared.inner.lock().unwrap();
                inner.seats[s] = SeatState {
                    player_id: Some(rec.player_id.clone()),
                    name: Some(rec.name.clone()),
                    kind: rec.kind.clone(),
                    house: rec.house,
                    stack: cfg.starting_stack,
                    stand_up: false,
                };
            }
        }
        let first_hand = self.hand_counter == 0;
        let (seats, sitting_out) = {
            let mut inner = self.shared.inner.lock().unwrap();
            let mut seats = Vec::new();
            let mut out = Vec::new();
            for (i, s) in inner.seats.iter_mut().enumerate() {
                let Some(pid) = s.player_id.clone() else { continue };
                if plan.dealt.contains(&i) {
                    let mut stack = match cfg.stack_mode {
                        StackMode::Reset => cfg.starting_stack,
                        StackMode::Carry if s.stack > 0 => s.stack,
                        StackMode::Carry => cfg.starting_stack,
                    };
                    if first_hand {
                        if let Some(v) = cfg.sandbox.as_ref().and_then(|b| b.stacks.as_ref()).and_then(|v| v.get(i)) {
                            stack = *v;
                        }
                    }
                    s.stack = stack;
                    seats.push(SeatInfo { seat: i, player_id: pid, name: s.name.clone().unwrap_or_default(), kind: s.kind.clone(), stack });
                } else {
                    out.push(SeatInfo { seat: i, player_id: pid, name: s.name.clone().unwrap_or_default(), kind: s.kind.clone(), stack: s.stack });
                }
            }
            inner.button_seat = Some(plan.button);
            inner.in_hand = true;
            inner.status = "running".into();
            (seats, out)
        };
        // Deck: seeded, optionally with a sandbox prefix.
        let mut dk = deck::deal_deck(&plan.seed, plan.deck_index);
        if cfg.is_sandbox() {
            if let Some(pre) = cfg.sandbox.as_ref().and_then(|s| s.decks.get(plan.deck_index as usize)) {
                let parsed: Option<Vec<u8>> = pre.iter().map(|c| cards::parse_card(c)).collect();
                match parsed {
                    Some(p) if p.iter().collect::<HashSet<_>>().len() == p.len() && p.len() <= 52 => dk = deck::with_prefix(&dk, &p),
                    _ => tracing::warn!("table {}: invalid sandbox deck prefix ignored", self.shared.id),
                }
            }
        }
        let owner = plan.match_id.clone().unwrap_or_else(|| self.shared.id.clone());
        let setup = HandSetup {
            table_id: self.shared.id.clone(),
            hand_id: format!("{owner}-h{}", plan.hand_no),
            match_id: plan.match_id.clone(),
            hand_no: plan.hand_no,
            deal_no: plan.deal_no,
            rotation: plan.rotation,
            cfg: cfg.clone(),
            button_seat: plan.button,
            seats,
            sitting_out,
            deck: dk,
            salt: random_hex(),
            started_ms,
            lean,
        };
        let hand_id = setup.hand_id.clone();
        let (mut runner, opening) = match HandRunner::new(setup) {
            Ok(x) => x,
            Err(e) => {
                self.shared.inner.lock().unwrap().in_hand = false;
                return Err(e);
            }
        };
        self.hand_counter += 1;
        {
            let mut inner = self.shared.inner.lock().unwrap();
            inner.hand_id = Some(hand_id.clone());
            if !lean {
                inner.holes = (0..runner.n()).filter_map(|i| runner.hole_cards_of_seat(runner.seat_of(i)).map(|c| (runner.seat_of(i), c))).collect();
            }
        }
        // House players.
        let seed64 = seed64(&plan.seed);
        let mut houses: Vec<Option<Box<dyn Player>>> = Vec::new();
        for idx in 0..runner.n() {
            let pid = runner.info[idx].player_id.clone();
            let is_house = self.player(&pid)?.house;
            let p = is_house
                .then(|| self.app.house.key_of(&pid))
                .flatten()
                .and_then(|k| self.app.house.get(&k))
                .map(|f| f.new_player(idx, &runner.ecfg, mix(mix(seed64, plan.hand_no as u64), idx as u64)));
            houses.push(p);
        }
        let has_remote = houses.iter().any(|h| h.is_none());
        let mut crn = cards::Rng::new(mix(seed64, plan.crn_key));
        let mut log: Vec<Value> = Vec::new();
        self.emit_evs(&hand_id, opening, &mut log, false);
        if !lean {
            self.publish_view(&runner);
        }

        while !runner.is_over() {
            let idx = runner.to_act_idx().unwrap();
            let seat = runner.seat_of(idx);
            let applied = if houses[idx].is_some() {
                if !lean {
                    let turn = runner.begin_turn(0, 0).unwrap().event.clone();
                    self.emit_evs(&hand_id, vec![turn], &mut log, false);
                }
                let u = crn.f64();
                let hole = runner.deal.holes[idx];
                let action = {
                    let view = View { cfg: &runner.ecfg, state: &runner.st, board: runner.board(), history: &runner.ehist, seat: idx };
                    houses[idx].as_mut().unwrap().act(&view, hole, u)
                };
                let (action, timeout) = if engine_legal(&runner.st, action) {
                    (action, false)
                } else {
                    runner.anomalies.push(format!("house bot at seat {seat} chose an illegal action {action:?}"));
                    (runner.timeout_action(), true)
                };
                runner.apply(action, timeout, 0)
            } else {
                match self.remote_decision(&mut runner, &hand_id, &mut log, plan.hide_cards).await {
                    Decision::Act(a, think) => runner.apply(a, false, think),
                    Decision::Timeout(think) => {
                        let a = runner.timeout_action();
                        runner.apply(a, true, think)
                    }
                }
            };
            for p in houses.iter_mut().flatten() {
                p.observe(&applied.pre, applied.idx, applied.action);
                for &l in &applied.reveals {
                    p.observe_board(&runner.deal.board[..l]);
                }
            }
            self.emit_evs(&hand_id, applied.events, &mut log, false);
            if !lean {
                self.publish_view(&runner);
            }
            if self.closed {
                break;
            }
        }
        if !runner.is_over() {
            // Table closed mid-hand: abandon without recording.
            let mut inner = self.shared.inner.lock().unwrap();
            inner.in_hand = false;
            inner.hand = None;
            return Ok(());
        }
        for p in houses.iter_mut().flatten() {
            p.finish(&runner.deal, &runner.ehist, &runner.st);
        }
        let (end_events, hist) = runner.finish(!lean || has_remote);
        self.emit_evs(&hand_id, end_events, &mut log, plan.hide_cards);
        // Settle seats.
        let mut gone = Vec::new();
        {
            let mut inner = self.shared.inner.lock().unwrap();
            for r in &hist.results {
                inner.seats[r.seat as usize].stack = r.stack_end;
            }
            for (i, s) in inner.seats.iter_mut().enumerate() {
                if s.stand_up && plan.assignment.is_none() {
                    *s = SeatState::empty();
                    gone.push(i);
                }
            }
            inner.in_hand = false;
            inner.hand = None;
            inner.hand_id = None;
            inner.holes.clear();
            inner.status = if self.plan.is_some() { "running".into() } else { "waiting".into() };
        }
        for i in gone {
            self.shared.emit_seat_update(self.hub(), i);
        }
        if self.plan.is_some() || self.app.cfg.save_free_hands {
            // Batched, off the hand loop; benchmark hands keep no event log.
            let events = if lean { Vec::new() } else { log };
            if let Some(rt) = &self.plan {
                let up = rt.record_hand(self.lane, &hist, self.app.store.as_ref());
                self.app.writer.push(hist, events);
                if let Some((d, progress)) = up {
                    self.shared.emit(self.hub(), None, "match_update", None, None, d);
                    if let (Some(p), Some(b)) = (progress, &rt.bench) {
                        let env = json!({"v": 1, "type": "benchmark_progress", "t_ms": now_ms(), "data": p});
                        self.app.hub.send(&b.player_id, &env.to_string());
                    }
                }
            } else {
                self.app.writer.push(hist, events);
            }
        } else if let Some(rt) = &self.plan {
            let _ = rt.record_hand(self.lane, &hist, self.app.store.as_ref());
        }
        if self.plan.is_none() && !lean {
            let delay = cfg.hand_delay_ms.unwrap_or(self.app.cfg.hand_delay_ms);
            if delay > 0 {
                sleep(Duration::from_millis(delay)).await;
            }
        }
        // Never starve other tasks on this worker when every seat is a house bot.
        tokio::task::yield_now().await;
        Ok(())
    }

    /// Ask a remote seat for its action; returns when it answers or its deadline passes.
    async fn remote_decision(&mut self, runner: &mut HandRunner, hand_id: &str, log: &mut Vec<Value>, hide: bool) -> Decision {
        let idx = runner.to_act_idx().unwrap();
        let pid = runner.info[idx].player_id.clone();
        let cfg_time = runner.setup.cfg.decision_time_ms;
        let bank0 = *self.banks.entry(pid.clone()).or_insert(runner.setup.cfg.time_bank_ms);
        let base = Duration::from_millis(cfg_time);
        let start = Instant::now();
        let hard = start + base + Duration::from_millis(bank0);
        let deadline_ms = now_ms() + cfg_time + bank0;
        let turn = runner.begin_turn(deadline_ms, bank0).unwrap().clone();
        let decision_id = turn.decision_id.clone();
        let env = self.emit_evs(hand_id, vec![turn.event.clone()], log, hide).remove(0);
        let env_str = env.to_string();

        let account = |this: &mut Self, elapsed: u64, timed_out: bool| {
            let over = elapsed.saturating_sub(cfg_time);
            let b = this.banks.entry(pid.clone()).or_insert(0);
            *b = if timed_out { 0 } else { b.saturating_sub(over) };
        };

        // Webhook seats are asked over HTTP instead of the socket.
        if self.app.cfg.enable_webhooks && !self.app.hub.connected(&pid) {
            if let Ok(rec) = self.player(&pid) {
                if let (Some(url), Some(secret)) = (rec.webhook_url.clone(), rec.webhook_secret.clone()) {
                    let remaining = hard.saturating_duration_since(Instant::now());
                    let res = crate::webhook::ask(url, secret, env_str.clone(), remaining).await;
                    let elapsed = start.elapsed().as_millis() as u64;
                    if let Ok((req, _)) = res {
                        if let Ok(a) = runner.validate(&req) {
                            account(self, elapsed, false);
                            return Decision::Act(a, elapsed);
                        }
                    }
                    account(self, elapsed, true);
                    return Decision::Timeout(elapsed);
                }
            }
        }

        let grace = Duration::from_millis(self.app.cfg.reconnect_grace_ms);
        let mut wait_until = if self.app.hub.connected(&pid) { hard } else { hard.min(Instant::now() + grace) };
        loop {
            let cmd = match timeout_at(wait_until, self.rx.recv()).await {
                Err(_) | Ok(None) => {
                    let elapsed = start.elapsed().as_millis() as u64;
                    account(self, elapsed, true);
                    return Decision::Timeout(elapsed);
                }
                Ok(Some(c)) => c,
            };
            match cmd {
                TableCmd::Close => {
                    self.closed = true;
                    let elapsed = start.elapsed().as_millis() as u64;
                    return Decision::Timeout(elapsed);
                }
                TableCmd::Leave { player_id } => self.leave(&player_id),
                TableCmd::Resume { player_id, conn } => {
                    if player_id == pid {
                        let _ = conn.send(env_str.clone());
                        wait_until = hard;
                    }
                }
                TableCmd::Act { player_id, data, conn } => {
                    let did = data["decision_id"].as_str().unwrap_or("").to_string();
                    if player_id != pid {
                        self.send_error(&conn, ApiError::conflict("not_your_turn", "it is not your turn"), Some(&did));
                        continue;
                    }
                    if data["hand_id"].as_str().map(|h| h != hand_id).unwrap_or(false) {
                        self.send_error(&conn, ApiError::conflict("stale_decision", "hand_id does not match the current hand"), Some(&did));
                        continue;
                    }
                    if did != decision_id {
                        if self.answered.contains(&did) {
                            continue; // duplicate act: idempotent
                        }
                        self.send_error(&conn, ApiError::conflict("stale_decision", "unknown or expired decision_id"), Some(&did));
                        continue;
                    }
                    let Some(kind) = data["type"].as_str() else {
                        self.send_error(&conn, ApiError::new(422, "illegal_action", "`type` is required"), Some(&did));
                        continue;
                    };
                    let req = ActReq { kind: kind.to_string(), to: data["to"].as_i64() };
                    match runner.validate(&req) {
                        Ok(a) => {
                            let elapsed = start.elapsed().as_millis() as u64;
                            self.answered.insert(did);
                            if self.answered.len() > 4096 {
                                self.answered.clear();
                            }
                            account(self, elapsed, false);
                            return Decision::Act(a, elapsed);
                        }
                        Err(e) => {
                            self.send_error(&conn, e, Some(&did));
                            // fresh your_turn with the same decision_id while time remains
                            self.app.hub.send(&pid, &env_str);
                        }
                    }
                }
            }
        }
    }
}

/// Drop hole cards, runout and deck from a `hand_end` payload (match `reveal: "match_end"`):
/// only the cards shown at showdown stay public.
pub(crate) fn redact_hand_end(d: &mut Value) {
    let shown: Vec<(String, Value)> = d["showdown"]
        .as_array()
        .map(|a| a.iter().filter(|x| x["shown"] == json!(true)).map(|x| (x["seat"].to_string(), x["cards"].clone())).collect())
        .unwrap_or_default();
    let mut m = serde_json::Map::new();
    for (k, v) in shown {
        m.insert(k, v);
    }
    d["hole_cards"] = Value::Object(m);
    d["runout"] = json!([]);
    d["deck_reveal"] = Value::Null;
}
