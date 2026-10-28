//! `extbridge` — plays SvanBot's REAL live decision policy (`sv10_policy::policy::decide_with`, default
//! `Params`, learning `ModelStore`) inside the Pluribus-reproduction benchmark harness.
//!
//! Protocol: JSON lines over stdin/stdout, as in pluribus `crates/bench/src/ext.rs`
//! (`scripts/extbots/rlcard_bot_server.py` is the reference). Two kinds of input line:
//!   * a decision request (`id`, `seat`, `hole`, `board`, `stacks`, `contrib`, `history`, `who`, ...)
//!     -> one reply line `{"id":N,"action":"fold|check|call|raise|allin","to":T}`;
//!   * an `observe` line (a finished hand with all hole cards) -> no reply; feeds the opponent models.
//!
//! The request is converted by REPLAYING the public history through SvanBot's own engine (`Hand`), so the
//! `Situation` (pot, bets, history records, min/max raise, positions) is exactly what its live/sim path
//! builds (`Situation::from_hand`). Opponent names are `e<entrant>s<seat>` so identical bots in different seats
//! keep separate, persistent profiles; our own seat is `me`.
//!
//! Env: `SVAN_STATS_DIR` (per-process stats json), `SVAN_LEARN=0` to freeze models (cold priors only).
//! `extbridge selftest` runs the conversion unit tests on hand-written spots.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Instant;
use sv10_core::cards::Card;
use sv10_core::engine::{Action, ActionKind, Hand};
use sv10_core::model::{HandSummary, ModelStore, aggressive};
use sv10_core::policy::{Params, decide_with};
use sv10_core::situation::Situation;
use sv10_rng::SeedableRng;
use sv10_rng::rngs::SmallRng;

#[derive(Default)]
struct EntStat {
    hands: u64,
    vpip: u64,
    pfr: u64,
}

#[derive(Default)]
struct Stats {
    decisions: u64,
    ms_total: f64,
    ms: Vec<f32>,
    replay_mismatch: u64,
    replay_err: u64,
    panics: u64,
    fold_when_free: u64,
    raise_unavailable: u64,
    observed: u64,
    observe_err: u64,
    /// my action counts by street name and kind
    acts: HashMap<String, u64>,
    ents: HashMap<usize, EntStat>,
    /// preflop first decision tallies for "me": hands seen / vpip / pfr (decision level)
    me_pre_hands: u64,
    me_pre_vpip: u64,
    me_pre_pfr: u64,
}

struct Bot {
    models: ModelStore,
    params: Params,
    learn: bool,
    stats: Stats,
    stats_path: Option<String>,
}

fn cards(v: &Value) -> Result<Vec<Card>, String> {
    v.as_array()
        .ok_or("cards not an array")?
        .iter()
        .map(|c| Card::parse(c.as_str().unwrap_or("")).ok_or_else(|| format!("bad card {c}")))
        .collect()
}

fn unused(used: &[Card], k: usize) -> Vec<Card> {
    (0..52u8).map(Card).filter(|c| !used.contains(c)).take(k).collect()
}

/// Replay `hist` (public events) through SvanBot's engine. Seat 0 = SB, 1 = BB, last = button (our harness).
fn build_hand(n: usize, sb: i64, bb: i64, start: &[i64], holes: &[[Card; 2]], runout: [Card; 5], hist: &[Value]) -> Result<Hand, String> {
    let seats = (0..n).map(|s| Hand::seat_state(start[s], holes[s])).collect();
    let mut h = Hand::with_cards(seats, runout, n - 1, sb, bb);
    for (i, e) in hist.iter().enumerate() {
        let seat = e["seat"].as_u64().ok_or("event seat")? as usize;
        if h.actor() != Some(seat) {
            return Err(format!("event {i}: engine actor {:?} != {seat}", h.actor()));
        }
        let a = match e["kind"].as_str().unwrap_or("") {
            "fold" => Action::Fold,
            "check" => Action::Check,
            "call" => Action::Call,
            "raise" => Action::RaiseTo(e["to"].as_i64().ok_or("raise to")?),
            k => return Err(format!("unknown kind {k}")),
        };
        h.apply(a).map_err(|m| format!("event {i} {e}: {m}"))?;
    }
    Ok(h)
}

fn names_for(me: usize, who: &[usize], n: usize) -> Vec<String> {
    (0..n)
        .map(|s| {
            if s == me {
                "me".to_string()
            } else if who.len() == n {
                format!("e{}s{}", who[s], s)
            } else {
                format!("s{s}")
            }
        })
        .collect()
}

fn who_of(r: &Value, n: usize) -> Vec<usize> {
    match r["who"].as_array() {
        Some(a) if a.len() == n => a.iter().map(|x| x.as_u64().unwrap_or(0) as usize).collect(),
        _ => Vec::new(),
    }
}

/// Request -> (SvanBot `Situation`, replay mismatch flag).
fn situation_from_request(r: &Value) -> Result<(Situation, bool), String> {
    let n = r["players"].as_u64().ok_or("players")? as usize;
    let me = r["seat"].as_u64().ok_or("seat")? as usize;
    let (sb, bb) = (r["sb"].as_i64().ok_or("sb")?, r["bb"].as_i64().ok_or("bb")?);
    let hole = cards(&r["hole"])?;
    let board = cards(&r["board"])?;
    let stacks: Vec<i64> = r["stacks"].as_array().ok_or("stacks")?.iter().map(|x| x.as_i64().unwrap_or(0)).collect();
    let contrib: Vec<i64> = r["contrib"].as_array().ok_or("contrib")?.iter().map(|x| x.as_i64().unwrap_or(0)).collect();
    let start: Vec<i64> = (0..n).map(|s| stacks[s] + contrib[s]).collect();
    let mut used: Vec<Card> = hole.clone();
    used.extend(board.iter().copied());
    let filler = unused(&used, 2 * n + 5);
    let mut it = filler.into_iter();
    let holes: Vec<[Card; 2]> = (0..n).map(|s| if s == me { [hole[0], hole[1]] } else { [it.next().unwrap(), it.next().unwrap()] }).collect();
    let mut runout: Vec<Card> = board.clone();
    while runout.len() < 5 {
        runout.push(it.next().unwrap());
    }
    let runout: [Card; 5] = [runout[0], runout[1], runout[2], runout[3], runout[4]];
    let hist = r["history"].as_array().ok_or("history")?;
    let h = build_hand(n, sb, bb, &start, &holes, runout, hist)?;
    // The replay must land exactly on the harness state.
    let mut bad = h.actor() != Some(me) || h.board != board || h.pot() != r["pot"].as_i64().unwrap_or(-1);
    for s in 0..n {
        bad |= h.seats[s].stack != stacks[s] || h.seats[s].invested != contrib[s];
    }
    let who = who_of(r, n);
    let names = names_for(me, &who, n);
    Ok((Situation::from_hand(&h, me, &names), bad))
}

fn action_json(id: u64, a: Action, sit: &Situation, st: &mut Stats) -> Value {
    match a {
        Action::Fold => {
            if sit.can_check {
                st.fold_when_free += 1;
            }
            json!({"id": id, "action": "fold"})
        }
        Action::Check => json!({"id": id, "action": "check"}),
        Action::Call => json!({"id": id, "action": "call"}),
        Action::RaiseTo(t) => {
            if sit.max_raise_to.is_none() {
                st.raise_unavailable += 1;
            }
            json!({"id": id, "action": "raise", "to": t})
        }
        Action::AllIn => {
            if sit.max_raise_to.is_none() {
                st.raise_unavailable += 1;
            }
            json!({"id": id, "action": "allin"})
        }
    }
}

impl Bot {
    fn decide(&mut self, r: &Value) -> Value {
        let id = r["id"].as_u64().unwrap_or(0);
        let t0 = Instant::now();
        let (sit, bad) = match situation_from_request(r) {
            Ok(x) => x,
            Err(e) => {
                self.stats.replay_err += 1;
                eprintln!("replay error: {e}");
                return json!({"id": id, "error": format!("replay: {e}")});
            }
        };
        if bad {
            self.stats.replay_mismatch += 1;
        }
        let seed = id.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (std::process::id() as u64);
        let res = catch_unwind(AssertUnwindSafe(|| {
            let mut rng = SmallRng::seed_from_u64(seed);
            decide_with(&sit, &self.models, &self.params, None, &mut rng)
        }));
        let d = match res {
            Ok(d) => d,
            Err(_) => {
                self.stats.panics += 1;
                return json!({"id": id, "error": "panic"});
            }
        };
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        self.stats.decisions += 1;
        self.stats.ms_total += ms;
        if self.stats.ms.len() < 200_000 {
            self.stats.ms.push(ms as f32);
        }
        let kind = match d.action {
            Action::Fold => "fold",
            Action::Check => "check",
            Action::Call => "call",
            Action::RaiseTo(_) => "raise",
            Action::AllIn => "allin",
        };
        *self.stats.acts.entry(format!("{}:{}", sit.street.name(), kind)).or_default() += 1;
        if sit.street == sv10_core::engine::Street::Preflop && !sit.history.iter().any(|h| h.seat == sit.hero_seat) {
            self.stats.me_pre_hands += 1;
            if matches!(d.action, Action::Call | Action::RaiseTo(_) | Action::AllIn) && !sit.can_check {
                self.stats.me_pre_vpip += 1;
            }
            if matches!(d.action, Action::RaiseTo(_) | Action::AllIn) {
                self.stats.me_pre_pfr += 1;
            }
        }
        action_json(id, d.action, &sit, &mut self.stats)
    }

    fn observe(&mut self, r: &Value) -> Result<(), String> {
        let n = r["players"].as_u64().ok_or("players")? as usize;
        let (sb, bb) = (r["sb"].as_i64().ok_or("sb")?, r["bb"].as_i64().ok_or("bb")?);
        let start: Vec<i64> = r["stacks"].as_array().ok_or("stacks")?.iter().map(|x| x.as_i64().unwrap_or(0)).collect();
        let hv = r["holes"].as_array().ok_or("holes")?;
        let mut holes = Vec::new();
        for h in hv {
            let c = cards(h)?;
            holes.push([c[0], c[1]]);
        }
        let board = cards(&r["board"])?;
        // full runout is only needed to deal streets the history reaches; pad with unused cards
        let mut used: Vec<Card> = holes.iter().flatten().copied().collect();
        used.extend(board.iter().copied());
        let mut runout: Vec<Card> = board.clone();
        let mut pad = unused(&used, 5).into_iter();
        while runout.len() < 5 {
            runout.push(pad.next().unwrap());
        }
        let runout: [Card; 5] = [runout[0], runout[1], runout[2], runout[3], runout[4]];
        let hist = r["history"].as_array().ok_or("history")?;
        let h = build_hand(n, sb, bb, &start, &holes, runout, hist)?;
        let who = who_of(r, n);
        let names: Vec<String> = (0..n).map(|s| if who.len() == n { format!("e{}s{}", who[s], s) } else { format!("s{s}") }).collect();
        let showdown = r["showdown"].as_bool().unwrap_or(false);
        let summary = HandSummary {
            players: names.iter().cloned().enumerate().collect(),
            button: h.button,
            bb,
            history: h.history.clone(),
            board: if showdown { board.clone() } else { h.board.clone() },
            stacks: h.seats.iter().enumerate().map(|(i, s)| (i, s.start_stack)).collect(),
            shown: if showdown { h.seats.iter().enumerate().filter(|(_, s)| !s.folded).map(|(i, s)| (i, s.hole)).collect() } else { vec![] },
        };
        // per-entrant VPIP / PFR (diagnostics only)
        for s in 0..n {
            let e = self.stats.ents.entry(if who.len() == n { who[s] } else { 99 }).or_default();
            e.hands += 1;
            let pre = h.history.iter().filter(|a| a.seat == s && a.street == sv10_core::engine::Street::Preflop);
            let (mut vp, mut pf) = (false, false);
            for a in pre {
                if matches!(a.kind, ActionKind::Call | ActionKind::Raise | ActionKind::AllIn) {
                    vp = true;
                }
                if aggressive(a) {
                    pf = true;
                }
            }
            e.vpip += vp as u64;
            e.pfr += pf as u64;
        }
        if self.learn {
            self.models.observe(&summary, None);
        }
        self.stats.observed += 1;
        Ok(())
    }

    fn dump(&self) {
        let Some(p) = &self.stats_path else { return };
        let s = &self.stats;
        let mut ms = s.ms.clone();
        ms.sort_by(|a, b| a.total_cmp(b));
        let q = |f: f64| if ms.is_empty() { 0.0 } else { ms[((ms.len() - 1) as f64 * f) as usize] };
        let ents: Value = s
            .ents
            .iter()
            .map(|(k, e)| (k.to_string(), json!({"hands": e.hands, "vpip": e.vpip as f64 / e.hands.max(1) as f64, "pfr": e.pfr as f64 / e.hands.max(1) as f64})))
            .collect::<serde_json::Map<_, _>>()
            .into();
        let v = json!({
            "pid": std::process::id(), "decisions": s.decisions, "ms_mean": s.ms_total / s.decisions.max(1) as f64,
            "ms_p50": q(0.5), "ms_p95": q(0.95), "ms_p99": q(0.99), "ms_max": q(1.0),
            "replay_mismatch": s.replay_mismatch, "replay_err": s.replay_err, "panics": s.panics,
            "fold_when_free": s.fold_when_free, "raise_unavailable": s.raise_unavailable,
            "observed_hands": s.observed, "observe_err": s.observe_err,
            "acts": s.acts, "entrants": ents,
            "me_preflop": {"hands": s.me_pre_hands, "vpip": s.me_pre_vpip as f64 / s.me_pre_hands.max(1) as f64, "pfr": s.me_pre_pfr as f64 / s.me_pre_hands.max(1) as f64},
            "learn": self.learn,
        });
        let _ = std::fs::write(p, v.to_string());
    }
}

fn new_bot() -> Bot {
    let dir = std::env::var("SVAN_STATS_DIR").ok();
    Bot {
        models: ModelStore::default(),
        params: Params::default(),
        learn: std::env::var("SVAN_LEARN").map(|v| v != "0").unwrap_or(true),
        stats: Stats::default(),
        stats_path: dir.map(|d| format!("{d}/w{}.json", std::process::id())),
    }
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("selftest") {
        selftest();
        return;
    }
    // One decision at a time per process; the harness runs many processes.
    let _ = rayon::ThreadPoolBuilder::new().num_threads(1).build_global();
    let mut bot = new_bot();
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    let mut last_dump = Instant::now();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(r) = serde_json::from_str::<Value>(&line) else {
            eprintln!("bad json line");
            continue;
        };
        if r.get("observe").is_some() {
            if let Err(e) = bot.observe(&r) {
                bot.stats.observe_err += 1;
                eprintln!("observe error: {e}");
            }
            continue;
        }
        if last_dump.elapsed().as_secs_f64() > 3.0 {
            bot.dump();
            last_dump = Instant::now();
        }
        let reply = bot.decide(&r);
        if writeln!(out, "{reply}").is_err() || out.flush().is_err() {
            break;
        }
    }
    bot.dump();
}

// ---------------------------------------------------------------------------------------------------------
// Conversion tests on hand-written spots. Request JSON is exactly what pluribus ext.rs `request_json` emits.

fn req(v: Value) -> Value {
    v
}

fn run_spot(name: &str, r: Value) -> (Situation, Value) {
    let (sit, bad) = situation_from_request(&r).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert!(!bad, "{name}: replay mismatch");
    let mut b = new_bot();
    let reply = b.decide(&r);
    println!("{name}: pot={} call={} can_check={} min_raise={:?} max_raise={:?} street={:?} -> {reply}", sit.pot, sit.call_amount, sit.can_check, sit.min_raise_to, sit.max_raise_to, sit.street);
    (sit, reply)
}

fn selftest() {
    let _ = rayon::ThreadPoolBuilder::new().num_threads(2).build_global();
    // 1. preflop open: UTG (seat 2) with AA, nothing yet. stacks after blinds.
    let (s, r) = run_spot(
        "open",
        req(json!({"id":1,"seat":2,"players":6,"hole":["As","Ah"],"board":[],"street":0,"pot":150,"sb":50,"bb":100,
          "stacks":[9950,9900,10000,10000,10000,10000],"street_bets":[50,100,0,0,0,0],"contrib":[50,100,0,0,0,0],
          "folded":[false,false,false,false,false,false],"allin":[false,false,false,false,false,false],"cur_bet":100,"call_amount":100,
          "legal":{"fold":true,"check":false,"raise":true,"min_raise_to":200,"max_raise_to":10000},"history":[]})),
    );
    assert_eq!(s.hero_seat, 2);
    assert_eq!(s.call_amount, 100);
    assert_eq!(s.min_raise_to, Some(200));
    assert_eq!(s.max_raise_to, Some(10000));
    assert_eq!(r["action"], "raise", "AA UTG must raise: {r}");
    assert!(r["to"].as_i64().unwrap() >= 200 && r["to"].as_i64().unwrap() <= 10000);

    // 2. facing a 3-bet: BTN (seat 5) opened to 250, SB (seat 0) 3-bets to 900; we are BTN with KK... actor is seat 5 again? Order: 2 f,3 f,4 f,5 raise 250, 0 raise 900, 1 fold, 5 to act.
    let hist = json!([
        {"seat":2,"street":0,"kind":"fold","to":0},{"seat":3,"street":0,"kind":"fold","to":0},{"seat":4,"street":0,"kind":"fold","to":0},
        {"seat":5,"street":0,"kind":"raise","to":250},{"seat":0,"street":0,"kind":"raise","to":900},{"seat":1,"street":0,"kind":"fold","to":0}]);
    let (s, r) = run_spot(
        "facing-3bet",
        req(json!({"id":2,"seat":5,"players":6,"hole":["Ks","Kh"],"board":[],"street":0,"pot":1250,"sb":50,"bb":100,
          "stacks":[9100,9900,10000,10000,10000,9750],"street_bets":[900,0,0,0,0,250],"contrib":[900,100,0,0,0,250],
          "folded":[false,true,true,true,true,false],"allin":[false,false,false,false,false,false],"cur_bet":900,"call_amount":650,
          "legal":{"fold":true,"check":false,"raise":true,"min_raise_to":1550,"max_raise_to":10000},"history":hist})),
    );
    assert_eq!(s.call_amount, 650);
    assert_eq!(s.min_raise_to, Some(1550));
    assert_ne!(r["action"], "fold", "KK vs 3bet must not fold: {r}");

    // 3. flop c-bet: BTN raised 250, BB (seat 1) called; flop A72r, BB checks -> we (BTN) act with AK.
    let hist = json!([
        {"seat":2,"street":0,"kind":"fold","to":0},{"seat":3,"street":0,"kind":"fold","to":0},{"seat":4,"street":0,"kind":"fold","to":0},
        {"seat":5,"street":0,"kind":"raise","to":250},{"seat":0,"street":0,"kind":"fold","to":0},{"seat":1,"street":0,"kind":"call","to":0},
        {"seat":1,"street":1,"kind":"check","to":0}]);
    let (s, r) = run_spot(
        "flop-cbet",
        req(json!({"id":3,"seat":5,"players":6,"hole":["Ac","Kd"],"board":["As","7h","2c"],"street":1,"pot":550,"sb":50,"bb":100,
          "stacks":[9950,9750,10000,10000,10000,9750],"street_bets":[0,0,0,0,0,0],"contrib":[50,250,0,0,0,250],
          "folded":[true,false,true,true,true,false],"allin":[false,false,false,false,false,false],"cur_bet":0,"call_amount":0,
          "legal":{"fold":false,"check":true,"raise":true,"min_raise_to":100,"max_raise_to":9750},"history":hist})),
    );
    assert!(s.can_check);
    assert_eq!(s.pot, 550);
    assert_eq!(s.board.len(), 3);
    assert!(matches!(r["action"].as_str(), Some("raise") | Some("check")), "{r}");

    // 4. all-in facing: river, BB shoves 2000 into 1000 pot, hero (BTN) has the nuts.
    let hist = json!([
        {"seat":2,"street":0,"kind":"fold","to":0},{"seat":3,"street":0,"kind":"fold","to":0},{"seat":4,"street":0,"kind":"fold","to":0},
        {"seat":5,"street":0,"kind":"raise","to":250},{"seat":0,"street":0,"kind":"fold","to":0},{"seat":1,"street":0,"kind":"call","to":0},
        {"seat":1,"street":1,"kind":"check","to":0},{"seat":5,"street":1,"kind":"raise","to":200},{"seat":1,"street":1,"kind":"call","to":0},
        {"seat":1,"street":2,"kind":"check","to":0},{"seat":5,"street":2,"kind":"check","to":0},
        {"seat":1,"street":3,"kind":"raise","to":9550}]);
    let (s, r) = run_spot(
        "river-allin",
        req(json!({"id":4,"seat":5,"players":6,"hole":["Ad","As"],"board":["Ah","Ac","2c","7d","9s"],"street":3,"pot":10500,"sb":50,"bb":100,
          "stacks":[9950,0,10000,10000,10000,9550],"street_bets":[0,9550,0,0,0,0],"contrib":[50,10000,0,0,0,450],
          "folded":[true,false,true,true,true,false],"allin":[false,true,false,false,false,false],"cur_bet":9550,"call_amount":9550,
          "legal":{"fold":true,"check":false,"raise":false,"min_raise_to":0,"max_raise_to":0},"history":hist})),
    );
    let _ = s;
    assert_eq!(r["action"], "call", "quad aces vs shove must call: {r}");

    // 5. observe a finished hand (fold-out) and replay.
    let mut b = new_bot();
    let obs = json!({"observe":1,"players":6,"stacks":[10000,10000,10000,10000,10000,10000],"sb":50,"bb":100,"button":5,
        "holes":[["2c","3d"],["4c","5d"],["As","Ah"],["7c","8d"],["9c","Td"],["Jc","Qd"]],"board":[],"showdown":false,
        "history":[{"seat":2,"street":0,"kind":"raise","to":250},{"seat":3,"street":0,"kind":"fold","to":0},{"seat":4,"street":0,"kind":"fold","to":0},
          {"seat":5,"street":0,"kind":"fold","to":0},{"seat":0,"street":0,"kind":"fold","to":0},{"seat":1,"street":0,"kind":"fold","to":0}],
        "who":[1,1,0,1,1,1]});
    b.observe(&obs).unwrap();
    assert_eq!(b.stats.observed, 1);
    assert!(b.models.players.contains_key("e0s2"));
    println!("selftest ok");
}
