//! Protocol parsing + replay on hand-crafted message sequences that follow
//! docs.openpoker.ai (no raw recordings existed in the old crate's benchmark JSONs; the live
//! smoke test writes a raw trace that `tests/fixtures/` can be extended with).

use bench::builtin::builtin;
use bench::policy::{Dist, Player, PolicyFactory, View};
use cards::Card;
use nlhe::{Action, GameConfig, State};
use openpoker::analyze::{analyze, load_trace, HandRecord};
use openpoker::session::{HuMode, PolicySpec, Session, SessionConfig};
use openpoker::trace::MemSink;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// ------------------------------------------------------------------ mocks

struct CallFactory {
    observed: Arc<AtomicUsize>,
    shove: bool,
}
struct CallPlayer {
    observed: Arc<AtomicUsize>,
    shove: bool,
}
impl PolicyFactory for CallFactory {
    fn name(&self) -> String {
        "mock-call".into()
    }
    fn new_player(&self, _seat: usize, _cfg: &GameConfig, _seed: u64) -> Box<dyn Player> {
        Box::new(CallPlayer { observed: self.observed.clone(), shove: self.shove })
    }
}
impl Player for CallPlayer {
    fn observe(&mut self, _pre: &State, _seat: usize, _a: Action) {
        self.observed.fetch_add(1, Ordering::SeqCst);
    }
    fn dist(&mut self, v: &View, _hole: [Card; 2]) -> Dist {
        let l = v.state.legal();
        if self.shove && l.can_raise {
            return Dist::pure(Action::RaiseTo(l.max_raise_to));
        }
        Dist::pure(if l.can_check { Action::Check } else { Action::Call })
    }
}

fn spec(embed6: bool, shove: bool) -> (PolicySpec, Arc<AtomicUsize>) {
    let obs = Arc::new(AtomicUsize::new(0));
    let s = PolicySpec {
        label: "mock-call".into(),
        main: Arc::new(CallFactory { observed: obs.clone(), shove }),
        embed6,
        fallback: builtin("tag").unwrap(),
        hu: HuMode::Fallback,
    };
    (s, obs)
}

fn cfg(hands: usize, auto_rebuy: bool) -> SessionConfig {
    SessionConfig { buy_in: 2000, target_hands: hands, seed: 5, auto_rebuy, raw: true, sizing: "tiny".into(), buckets: "coarse".into(), max_stack_bb: 0.0 }
}

struct H {
    s: Session,
    sink: MemSink,
}
impl H {
    fn new(spec: PolicySpec, hands: usize) -> H {
        let sink = MemSink::default();
        H { s: Session::new(cfg(hands, true), spec, Box::new(sink.clone())), sink }
    }
    fn with_cap(spec: PolicySpec, hands: usize, cap_bb: f64) -> H {
        let sink = MemSink::default();
        let mut c = cfg(hands, true);
        c.max_stack_bb = cap_bb;
        H { s: Session::new(c, spec, Box::new(sink.clone())), sink }
    }
    fn send(&mut self, m: Value) -> Vec<Value> {
        self.s.on_message(&m)
    }
    fn bad_anomalies(&self) -> Vec<Value> {
        self.sink
            .of_kind("anomaly")
            .into_iter()
            .filter(|a| {
                matches!(
                    a["what"].as_str().unwrap_or(""),
                    "desync" | "replay_failed_will_rebuild" | "engine_server_mismatch" | "illegal_choice_replaced" | "profit_mismatch" | "your_turn_without_hand"
                )
            })
            .collect()
    }
    fn hands(&self) -> Vec<HandRecord> {
        let text: String = self.sink.records().iter().map(|r| r.to_string() + "\n").collect();
        load_trace(&text).0
    }
}

fn pa(seat: usize, action: &str, amount: Option<i64>, street: &str, stack: i64, pot: i64) -> Value {
    json!({"type":"player_action","seat":seat,"name":format!("bot{seat}"),"action":action,"amount":amount,"street":street,"stack":stack,"pot":pot})
}

fn players(stacks: [i64; 6], status: [&str; 6]) -> Value {
    Value::Array((0..6).map(|i| json!({"seat": i, "name": format!("bot{i}"), "stack": stacks[i], "status": status[i]})).collect())
}

fn table_joined() -> Value {
    json!({"type":"table_joined","table_id":"t1","seat":2,"players":(0..6).map(|i| json!({"seat":i,"name":format!("bot{i}"),"stack":2000})).collect::<Vec<_>>()})
}

fn hand_start(id: &str) -> Value {
    json!({"type":"hand_start","hand_id":id,"seat":2,"dealer_seat":0,"blinds":{"small_blind":10,"big_blind":20}})
}

fn your_turn(id: &str, tok: &str, valid: Value, pot: i64, board: &[&str], players: Value) -> Value {
    json!({"type":"your_turn","hand_id":id,"turn_token":tok,"valid_actions":valid,"pot":pot,"community_cards":board,"players":players,"min_raise":null,"max_raise":null})
}

/// The canonical 6-max hand: hero BB seat 2 calls a HJ open, checks the flop, calls a turn bet.
/// Returns the action messages we sent.
fn play_canonical_hand(h: &mut H, id: &str) -> Vec<Value> {
    let mut sent = Vec::new();
    h.send(hand_start(id));
    h.send(json!({"type":"hole_cards","cards":["Ah","Ad"]}));
    h.send(pa(3, "fold", None, "preflop", 2000, 30));
    h.send(pa(4, "raise", Some(60), "preflop", 1940, 90));
    h.send(pa(5, "fold", None, "preflop", 2000, 90));
    h.send(pa(0, "fold", None, "preflop", 2000, 90));
    h.send(pa(1, "fold", None, "preflop", 1990, 90));
    let pl = players([2000, 1990, 1980, 2000, 1940, 2000], ["folded", "folded", "active", "folded", "active", "folded"]);
    let out = h.send(your_turn(
        id,
        &format!("{id}-t1"),
        json!([{"action":"fold"},{"action":"call","amount":40},{"action":"raise","min":100,"max":2000}]),
        90,
        &[],
        pl,
    ));
    assert_eq!(out.len(), 1, "one action per turn");
    assert_eq!(out[0]["action"], "call");
    sent.push(out[0].clone());
    h.send(json!({"type":"action_ack","client_action_id":out[0]["client_action_id"],"status":"accepted"}));
    h.send(pa(2, "call", Some(40), "preflop", 1940, 130));
    h.send(json!({"type":"community_cards","cards":["2c","7d","9h"],"street":"flop"}));
    let st = ["folded", "folded", "active", "folded", "active", "folded"];
    let pl_at = |cur: [i64; 6]| players(cur, st);
    let pl2 = pl_at([2000, 1990, 1940, 2000, 1940, 2000]);
    let out = h.send(your_turn(
        id,
        &format!("{id}-t2"),
        json!([{"action":"check"},{"action":"raise","min":20,"max":1940}]),
        130,
        &["2c", "7d", "9h"],
        pl2.clone(),
    ));
    assert_eq!(out[0]["action"], "check");
    sent.push(out[0].clone());
    h.send(pa(2, "check", None, "flop", 1940, 130));
    h.send(pa(4, "check", None, "flop", 1940, 130));
    h.send(json!({"type":"community_cards","cards":["Qs"],"street":"turn"}));
    let out = h.send(your_turn(
        id,
        &format!("{id}-t3"),
        json!([{"action":"check"},{"action":"raise","min":20,"max":1940}]),
        130,
        &["2c", "7d", "9h", "Qs"],
        pl2.clone(),
    ));
    assert_eq!(out[0]["action"], "check");
    sent.push(out[0].clone());
    h.send(pa(2, "check", None, "turn", 1940, 130));
    h.send(pa(4, "raise", Some(100), "turn", 1840, 230));
    let out = h.send(your_turn(
        id,
        &format!("{id}-t4"),
        json!([{"action":"fold"},{"action":"call","amount":100},{"action":"raise","min":200,"max":1940}]),
        230,
        &["2c", "7d", "9h", "Qs"],
        pl_at([2000, 1990, 1940, 2000, 1840, 2000]),
    ));
    assert_eq!(out[0]["action"], "call");
    sent.push(out[0].clone());
    // Echo of our call is NOT sent here: the session must synthesise it before the next event.
    h.send(json!({"type":"community_cards","cards":["3d"],"street":"river"}));
    let out = h.send(your_turn(
        id,
        &format!("{id}-t5"),
        json!([{"action":"check"},{"action":"raise","min":20,"max":1840}]),
        330,
        &["2c", "7d", "9h", "Qs", "3d"],
        pl_at([2000, 1990, 1840, 2000, 1840, 2000]),
    ));
    assert_eq!(out[0]["action"], "check");
    sent.push(out[0].clone());
    h.send(pa(2, "check", None, "river", 1840, 330));
    h.send(pa(4, "check", None, "river", 1840, 330));
    let out = h.send(json!({
        "type":"hand_result","hand_id":id,"pot":330,"total_pot":330,
        "winners":[{"seat":2,"name":"bot2","amount":170,"stack":2170,"hand_description":"Pair of Aces"}],
        "payouts":[{"seat":2,"amount":330}],
        "shown_cards":{"2":["Ah","Ad"],"4":["Ks","Kc"]},
        "final_stacks":{"0":2000,"1":1990,"2":2170,"3":2000,"4":1840,"5":2000}
    }));
    assert!(out.is_empty() || out[0]["type"] == "leave_table");
    sent
}

// ------------------------------------------------------------------ tests

#[test]
fn canonical_hand_replays_cleanly() {
    let (sp, obs) = spec(false, false);
    let mut h = H::new(sp, 100);
    h.send(json!({"type":"connected","agent_id":"a","name":"me"}));
    h.send(table_joined());
    play_canonical_hand(&mut h, "h1");
    assert!(h.bad_anomalies().is_empty(), "{:#?}", h.bad_anomalies());
    let hands = h.hands();
    assert_eq!(hands.len(), 1);
    let r = &hands[0];
    assert_eq!(r.profit, Some(170));
    assert_eq!(r.profit_src, "final_stacks");
    assert_eq!(r.position, "BB");
    assert_eq!(r.n_players, 6);
    assert_eq!(r.board.len(), 5);
    assert_eq!(r.lock_len, Some(5));
    assert!(r.showdown && !r.desync);
    assert_eq!(r.decisions, 5);
    assert_eq!(r.fallbacks, 0);
    assert_eq!(r.issues, 0, "engine and server must agree: {:#?}", h.sink.of_kind("decision").iter().map(|d| d["issues"].clone()).collect::<Vec<_>>());
    let hero = r.players.iter().find(|p| p.seat == 2).unwrap();
    assert_eq!((hero.start_stack, hero.contrib), (Some(2000), Some(160)));
    assert!(obs.load(Ordering::SeqCst) >= 5);
    assert_eq!(h.s.counters.hands_done, 1);
    assert_eq!(h.s.counters.acks, 1);
    assert!(!h.sink.of_kind("raw").is_empty());
    let d = &h.sink.of_kind("decision")[0];
    assert_eq!(d["chosen"], "call");
    assert_eq!(d["sent"], "call");
    assert_eq!(d["dist"][0][0], "call");
}

#[test]
fn analyzer_runs_on_session_trace() {
    let (sp, _) = spec(false, false);
    let mut h = H::new(sp, 100);
    h.send(table_joined());
    play_canonical_hand(&mut h, "h1");
    let text: String = h.sink.records().iter().map(|r| r.to_string() + "\n").collect();
    let (hands, sum) = load_trace(&text);
    assert_eq!(hands.len(), 1);
    assert_eq!(sum.decisions, 5);
    let r = analyze(&hands, 100);
    assert!((r.raw.mean_bb100 - 170.0 / 20.0 * 100.0).abs() < 1e-9);
    assert_eq!(r.by_position[0].key, "BB");
    assert!(r.by_opponent.iter().any(|g| g.key == "bot4"));
}

#[test]
fn target_hands_reached_sends_leave_table() {
    let (sp, _) = spec(false, false);
    let mut h = H::new(sp, 1);
    h.send(table_joined());
    h.send(hand_start("h1"));
    h.send(json!({"type":"hole_cards","cards":["7h","2d"]}));
    let out = h.send(json!({"type":"hand_result","hand_id":"h1","winners":[],"final_stacks":{"2":1980}}));
    assert!(h.s.done);
    assert!(out.iter().any(|m| m["type"] == "leave_table"));
}

#[test]
fn short_handed_blueprint_embedding_feeds_phantom_folds() {
    // 4 players: seats 0 (BTN), 1 (SB), 2 (hero BB), 4 (UTG-equivalent). Dealer 0.
    let (sp, obs) = spec(true, false);
    let mut h = H::new(sp, 100);
    h.send(json!({"type":"table_joined","table_id":"t","seat":2,"players":[
        {"seat":0,"stack":2000},{"seat":1,"stack":2000},{"seat":2,"stack":2000},{"seat":4,"stack":2000}]}));
    h.send(hand_start("h1"));
    h.send(json!({"type":"hole_cards","cards":["Kh","Kd"]}));
    h.send(pa(4, "raise", Some(60), "preflop", 1940, 90));
    h.send(pa(0, "fold", None, "preflop", 2000, 90));
    h.send(pa(1, "fold", None, "preflop", 1990, 90));
    let pl = json!([
        {"seat":0,"stack":2000,"status":"folded"},{"seat":1,"stack":1990,"status":"folded"},
        {"seat":2,"stack":1980,"status":"active"},{"seat":4,"stack":1940,"status":"active"},
        {"seat":3,"stack":0,"status":"sitting_out"}
    ]);
    let out = h.send(your_turn(
        "h1",
        "tk",
        json!([{"action":"fold"},{"action":"call","amount":40},{"action":"raise","min":100,"max":2000}]),
        90,
        &[],
        pl,
    ));
    assert_eq!(out[0]["action"], "call");
    assert!(h.bad_anomalies().is_empty(), "{:#?}", h.bad_anomalies());
    // 2 phantom folds (6-4) + the 3 real actions before the hero acts.
    assert_eq!(obs.load(Ordering::SeqCst), 2 + 3);
    let d = &h.sink.of_kind("decision")[0];
    assert_eq!(d["meta"]["n_players"], 4);
    assert_eq!(d["meta"]["position"], "BB");
}

#[test]
fn heads_up_uses_fallback_policy_on_native_engine() {
    let (sp, obs) = spec(true, false);
    let mut h = H::new(sp, 100);
    h.send(json!({"type":"table_joined","table_id":"t","seat":1,"players":[{"seat":0,"stack":2000},{"seat":1,"stack":2000}]}));
    // Open Poker heads-up: dealer 0 is the BB, so hero (seat 1) is the SB and acts first.
    h.send(json!({"type":"hand_start","hand_id":"hu","seat":1,"dealer_seat":0,"blinds":{"small_blind":10,"big_blind":20}}));
    h.send(json!({"type":"hole_cards","cards":["Ah","Kd"]}));
    let out = h.send(your_turn(
        "hu",
        "tk",
        json!([{"action":"fold"},{"action":"call","amount":10},{"action":"raise","min":40,"max":2000},{"action":"all_in","amount":2000}]),
        30,
        &[],
        json!([{"seat":0,"stack":1980},{"seat":1,"stack":1990}]),
    ));
    assert_eq!(out.len(), 1);
    assert!(h.bad_anomalies().is_empty(), "{:#?}", h.bad_anomalies());
    let d = &h.sink.of_kind("decision")[0];
    assert!(d["meta"]["policy"].as_str().unwrap().starts_with("fallback:"));
    assert_eq!(d["meta"]["position"], "BTN/SB");
    assert_eq!(obs.load(Ordering::SeqCst), 0, "main policy must not see heads-up hands");
    assert!(d["issues"].as_array().unwrap().is_empty(), "{d}");
}

#[test]
fn all_in_runout_records_lock_and_hands_are_adjustable() {
    // Hero shoves preflop with AA (mock shoves), villain (seat 4) calls all-in, board runs out.
    let (sp, _) = spec(false, true);
    let mut h = H::new(sp, 100);
    h.send(table_joined());
    h.send(hand_start("allin"));
    h.send(json!({"type":"hole_cards","cards":["Ah","Ad"]}));
    h.send(pa(3, "fold", None, "preflop", 2000, 30));
    h.send(pa(4, "raise", Some(60), "preflop", 1940, 90));
    h.send(pa(5, "fold", None, "preflop", 2000, 90));
    h.send(pa(0, "fold", None, "preflop", 2000, 90));
    h.send(pa(1, "fold", None, "preflop", 1990, 90));
    let pl = players([2000, 1990, 1980, 2000, 1940, 2000], ["folded", "folded", "active", "folded", "active", "folded"]);
    let out = h.send(your_turn(
        "allin",
        "tk1",
        json!([{"action":"fold"},{"action":"call","amount":40},{"action":"raise","min":100,"max":2000}]),
        90,
        &[],
        pl,
    ));
    assert_eq!(out[0]["action"], "raise");
    assert_eq!(out[0]["amount"], 2000, "mock shoves: raise-to = stack + street bet");
    h.send(pa(2, "raise", Some(2000), "preflop", 0, 2070));
    h.send(pa(4, "call", None, "preflop", 0, 4010));
    h.send(json!({"type":"community_cards","cards":["2c","7d","9h"],"street":"flop"}));
    h.send(json!({"type":"community_cards","cards":["Js"],"street":"turn"}));
    h.send(json!({"type":"community_cards","cards":["3d"],"street":"river"}));
    h.send(json!({
        "type":"hand_result","hand_id":"allin","winners":[{"seat":2,"amount":2010}],"payouts":[{"seat":2,"amount":4010}],
        "shown_cards":[{"seat":2,"cards":["Ah","Ad"]},{"seat":4,"cards":["Ks","Kc"]}],
        "final_stacks":[{"seat":2,"stack":4010},{"seat":4,"stack":0},{"seat":0,"stack":2000},{"seat":1,"stack":1990},{"seat":3,"stack":2000},{"seat":5,"stack":2000}]
    }));
    assert!(h.bad_anomalies().is_empty(), "{:#?}", h.bad_anomalies());
    let hands = h.hands();
    assert_eq!(hands.len(), 1);
    let r = &hands[0];
    assert_eq!(r.lock_len, Some(0), "all-in before the flop");
    assert_eq!(r.profit, Some(2010));
    // AA vs KK all-in preflop: ~82% of a 4010 pot minus the 2000 invested.
    let ev = openpoker::analyze::allin_ev(r, 40_000).expect("cards known -> adjustable");
    assert!(ev > 1100.0 && ev < 1500.0, "ev {ev}");
}

#[test]
fn duplicates_and_stale_sequence_numbers_are_ignored() {
    let (sp, _) = spec(false, false);
    let mut h = H::new(sp, 100);
    h.send(table_joined());
    h.send(json!({"type":"player_joined","seat":3,"name":"x","stack":2000,"table_seq":10}));
    let out = h.send(json!({"type":"player_left","seat":3,"table_seq":9}));
    assert!(out.is_empty());
    h.send(hand_start("d1"));
    h.send(json!({"type":"hole_cards","cards":["Ah","Ad"]}));
    h.send(pa(3, "fold", None, "preflop", 2000, 30));
    h.send(pa(4, "raise", Some(60), "preflop", 1940, 90));
    h.send(pa(5, "fold", None, "preflop", 2000, 90));
    h.send(pa(0, "fold", None, "preflop", 2000, 90));
    h.send(pa(1, "fold", None, "preflop", 1990, 90));
    let mk = || {
        your_turn(
            "d1",
            "same-token",
            json!([{"action":"fold"},{"action":"call","amount":40},{"action":"raise","min":100,"max":2000}]),
            90,
            &[],
            players([2000, 1990, 1980, 2000, 1940, 2000], ["folded", "folded", "active", "folded", "active", "folded"]),
        )
    };
    assert_eq!(h.send(mk()).len(), 1);
    assert_eq!(h.send(mk()).len(), 0, "same turn_token delivered twice must not act twice");
}

#[test]
fn rejection_is_retried_once_with_a_safe_action() {
    let (sp, _) = spec(false, true);
    let mut h = H::new(sp, 100);
    h.send(table_joined());
    h.send(hand_start("r1"));
    h.send(json!({"type":"hole_cards","cards":["Ah","Ad"]}));
    h.send(pa(3, "fold", None, "preflop", 2000, 30));
    h.send(pa(4, "raise", Some(60), "preflop", 1940, 90));
    h.send(pa(5, "fold", None, "preflop", 2000, 90));
    h.send(pa(0, "fold", None, "preflop", 2000, 90));
    h.send(pa(1, "fold", None, "preflop", 1990, 90));
    let out = h.send(your_turn(
        "r1",
        "tk",
        json!([{"action":"fold"},{"action":"call","amount":40},{"action":"raise","min":100,"max":2000}]),
        90,
        &[],
        players([2000, 1990, 1980, 2000, 1940, 2000], ["folded", "folded", "active", "folded", "active", "folded"]),
    ));
    assert_eq!(out[0]["action"], "raise");
    let retry = h.send(json!({"type":"error","code":"invalid_action","message":"nope"}));
    assert_eq!(retry.len(), 1);
    assert_eq!(retry[0]["action"], "fold", "passive fallback when facing a bet and check is not offered");
    assert_ne!(retry[0]["client_action_id"], out[0]["client_action_id"]);
    assert_eq!(retry[0]["turn_token"], "tk");
    let again = h.send(json!({"type":"error","code":"invalid_action"}));
    assert!(again.is_empty(), "only one retry");
    assert_eq!(h.s.counters.rejects, 2);
    let stale = h.send(json!({"type":"action_rejected","code":"stale_turn_token"}));
    assert!(stale.is_empty());
}

#[test]
fn busted_without_auto_rebuy_rebuys_then_rejoins() {
    let (sp, _) = spec(false, false);
    let sink = MemSink::default();
    let mut s = Session::new(cfg(10, false), sp, Box::new(sink));
    let out = s.on_message(&json!({"type":"busted","options":["rebuy","leave"]}));
    assert_eq!(out[0]["type"], "rebuy");
    let out = s.on_message(&json!({"type":"rebuy_confirmed","new_stack":1500}));
    assert_eq!(out[0]["type"], "join_lobby");
    assert_eq!(out[0]["buy_in"], 2000);
    let out = s.on_message(&json!({"type":"table_closed","reason":"x"}));
    assert_eq!(out[0]["type"], "join_lobby");
}

#[test]
fn busted_with_chips_left_rejoins_when_rebuy_is_refused() {
    // Live: after a bust the season balance still held chips, so the server answered our
    // rebuy with `cannot_rebuy` — we must rejoin a table instead of idling.
    let (sp, _) = spec(false, false);
    let sink = MemSink::default();
    let mut s = Session::new(cfg(10, false), sp, Box::new(sink));
    let out = s.on_message(&json!({"type":"busted","options":["rebuy","leave"]}));
    assert_eq!(out[0]["type"], "rebuy");
    let out = s.on_message(&json!({"type":"error","code":"cannot_rebuy","message":"Cannot rebuy — you still have chips."}));
    assert_eq!(out[0]["type"], "join_lobby");
    assert_eq!(out[0]["buy_in"], 2000);
}

#[test]
fn resync_snapshot_restores_turn() {
    // Warm reconnect: the server replays the hand's events, then a snapshot with our turn token.
    let (sp, _) = spec(false, false);
    let mut h = H::new(sp, 100);
    h.send(table_joined());
    let events = vec![
        hand_start("rs"),
        json!({"type":"hole_cards","cards":["Ah","Ad"]}),
        pa(3, "fold", None, "preflop", 2000, 30),
        pa(4, "raise", Some(60), "preflop", 1940, 90),
        pa(5, "fold", None, "preflop", 2000, 90),
        pa(0, "fold", None, "preflop", 2000, 90),
        pa(1, "fold", None, "preflop", 1990, 90),
    ];
    let out = h.send(json!({
        "type":"resync_response","role":"player","from_table_seq":1,"to_table_seq":8,
        "replayed_events": events,
        "snapshot":{
            "street":"preflop","dealer_seat":0,"pot":90,"actor_seat":2,"to_call":40,"min_raise_to":100,"max_raise_to":2000,
            "board":[],
            "seats":[
                {"seat":0,"stack":2000,"bet":0,"status":"folded","in_hand":false},
                {"seat":1,"stack":1990,"bet":10,"status":"folded","in_hand":false},
                {"seat":2,"stack":1980,"bet":20,"status":"active","in_hand":true},
                {"seat":3,"stack":2000,"bet":0,"status":"folded","in_hand":false},
                {"seat":4,"stack":1940,"bet":60,"status":"active","in_hand":true},
                {"seat":5,"stack":2000,"bet":0,"status":"folded","in_hand":false}
            ],
            "hero":{"seat":2,"hole_cards":["Ah","Ad"],"turn_token":"resync-tok","valid_actions":[{"action":"fold"},{"action":"call","amount":40},{"action":"raise","min":100,"max":2000}]}
        }
    }));
    assert_eq!(out.len(), 1, "snapshot with our turn must produce exactly one action");
    assert_eq!(out[0]["turn_token"], "resync-tok");
    assert_eq!(out[0]["action"], "call");
    assert!(h.bad_anomalies().is_empty(), "{:#?}", h.bad_anomalies());
    assert_eq!(h.s.counters.fallbacks, 0);
}

#[test]
fn unplaceable_action_falls_back_to_passive_and_flags_desync() {
    let (sp, _) = spec(false, false);
    let mut h = H::new(sp, 100);
    h.send(table_joined());
    h.send(hand_start("bad"));
    h.send(json!({"type":"hole_cards","cards":["Ah","Ad"]}));
    h.send(pa(3, "fold", None, "preflop", 2000, 30));
    // Seat 5 acts before seat 4: out of turn.
    h.send(pa(5, "raise", Some(60), "preflop", 1940, 90));
    h.send(pa(4, "fold", None, "preflop", 2000, 90));
    let out = h.send(your_turn(
        "bad",
        "tk",
        json!([{"action":"fold"},{"action":"call","amount":40},{"action":"raise","min":100,"max":2000}]),
        90,
        &[],
        players([2000, 1990, 1980, 2000, 2000, 1940], ["active", "active", "active", "folded", "folded", "active"]),
    ));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["action"], "fold", "safe action when facing a bet");
    assert_eq!(h.s.counters.fallbacks, 1);
    assert_eq!(h.s.counters.decisions, 1);
    let d = &h.sink.of_kind("decision")[0];
    assert!(d["fallback"].is_string());
    h.send(json!({"type":"hand_result","hand_id":"bad","winners":[],"final_stacks":{"2":1980}}));
    assert!(h.hands()[0].desync);
}

#[test]
fn history_fixture_loads_and_reconstructs() {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/history_sample.json")).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    let n = v["hands"].as_array().unwrap().len();
    let recs = openpoker::analyze::load_history(&v);
    assert_eq!(recs.len(), n);
    let total: i64 = v["hands"].as_array().unwrap().iter().map(|h| h["profit"].as_i64().unwrap()).sum();
    assert_eq!(recs.iter().filter_map(|r| r.profit).sum::<i64>(), total);
    let rep = analyze(&recs, 1000);
    assert_eq!(rep.hands, n);
    assert!(rep.render().contains("bb/100"));
    // The export is lossy (street labels, call amounts, pot_total): only clean hands reconstruct.
    let ok = recs.iter().filter(|r| !r.players.is_empty() && !r.desync).count();
    assert!(ok >= 3, "only {ok}/{n} history hands reconstructed");
}

// ------------------------------------------------------------------ real recording

/// Plays back the actions of a recorded decision list, ignoring what the engine says.
struct Scripted {
    queue: Arc<std::sync::Mutex<std::collections::VecDeque<Action>>>,
}
struct ScriptedPlayer {
    queue: Arc<std::sync::Mutex<std::collections::VecDeque<Action>>>,
}
impl PolicyFactory for Scripted {
    fn name(&self) -> String {
        "scripted".into()
    }
    fn new_player(&self, _seat: usize, _cfg: &GameConfig, _seed: u64) -> Box<dyn Player> {
        Box::new(ScriptedPlayer { queue: self.queue.clone() })
    }
}
impl Player for ScriptedPlayer {
    fn dist(&mut self, _v: &View, _hole: [Card; 2]) -> Dist {
        Dist::pure(self.queue.lock().unwrap().pop_front().expect("more decisions than recorded"))
    }
}

/// `tests/fixtures/smoke_raw.jsonl` is a real 10-hand session (builtin:tag, 2026-10-13, 3-6
/// handed tables, names redacted). Replaying every inbound message with a policy that repeats
/// the recorded decisions must reproduce the recorded outbound actions exactly, with a clean
/// engine replay (no desync, no engine/server disagreement) and `final_stacks`-based profits.
#[test]
fn recorded_live_session_replays_exactly() {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/smoke_raw.jsonl")).unwrap();
    let rows: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let mut queue = std::collections::VecDeque::new();
    let mut recorded_out = Vec::new();
    for r in &rows {
        if r["dir"] == "out" && r["msg"]["type"] == "action" {
            let m = &r["msg"];
            let a = match m["action"].as_str().unwrap() {
                "fold" => Action::Fold,
                "check" => Action::Check,
                "call" => Action::Call,
                "raise" => Action::RaiseTo(m["amount"].as_i64().unwrap() as i32 * 5),
                other => panic!("unexpected recorded action {other}"),
            };
            queue.push_back(a);
            recorded_out.push((m["action"].clone(), m["amount"].clone(), m["turn_token"].clone()));
        }
    }
    let n_results = rows.iter().filter(|r| r["dir"] == "in" && r["msg"]["type"] == "hand_result").count();
    assert!(n_results >= 10, "the recording ends with at least 10 results");
    let sp = PolicySpec {
        label: "scripted".into(),
        main: Arc::new(Scripted { queue: Arc::new(std::sync::Mutex::new(queue)) }),
        embed6: true,
        fallback: builtin("tag").unwrap(),
        hu: HuMode::Policy,
    };
    let mut h = H::new(sp, 10);
    let mut sent = Vec::new();
    for r in rows.iter().filter(|r| r["dir"] == "in") {
        for m in h.send(r["msg"].clone()) {
            if m["type"] == "action" {
                sent.push((m["action"].clone(), m["amount"].clone(), m["turn_token"].clone()));
            }
        }
    }
    assert_eq!(sent.len(), recorded_out.len());
    for (i, (a, b)) in sent.iter().zip(&recorded_out).enumerate() {
        // The recording has no `amount` key for non-raises; ours is JSON null in the same spot.
        assert_eq!((&a.0, &a.2), (&b.0, &b.2), "decision {i}");
        assert!(a.1 == b.1 || (a.1.is_null() && b.1.is_null()), "decision {i}: amount {} vs {}", a.1, b.1);
    }
    assert!(h.bad_anomalies().is_empty(), "{:#?}", h.bad_anomalies());
    let hands = h.hands();
    assert!(hands.len() >= 10);
    for r in &hands {
        assert!(!r.desync, "{}", r.hand_id);
        assert_eq!(r.issues, 0, "{}: {:#?}", r.hand_id, h.sink.of_kind("decision").iter().filter(|d| d["hand_id"] == r.hand_id.as_str()).map(|d| d["issues"].clone()).collect::<Vec<_>>());
        assert_eq!(r.profit_src, "final_stacks");
        assert!(r.profit.is_some() && r.bb == 20);
    }
    assert!(h.s.done);
    assert_eq!(h.s.counters.fallbacks, 0);
    // The report over a real trace renders.
    let rep = analyze(&hands, 2000);
    assert_eq!(rep.hands, hands.len());
    // Every showdown hand with cards for all live players must be adjustable or river-locked.
    for r in hands.iter().filter(|r| r.showdown) {
        assert!(r.lock_len.is_some(), "showdown hand without lock info: {}", r.hand_id);
    }
}

/// A real (toy-trained) 6-player blueprint through the short-handed embedding: the policy sees
/// the phantom folds as part of its abstract history and answers from its tables.
#[test]
fn toy_blueprint_plays_a_four_handed_embedded_hand() {
    let bp = Arc::new(bench::bp::train_toy(6, 20_000, 3));
    let sp = PolicySpec { label: "toy-bp".into(), main: bp.clone(), embed6: true, fallback: builtin("tag").unwrap(), hu: HuMode::Fallback };
    let mut h = H::new(sp, 100);
    h.send(json!({"type":"table_joined","table_id":"t","seat":2,"players":[
        {"seat":0,"stack":2000},{"seat":1,"stack":2000},{"seat":2,"stack":2000},{"seat":4,"stack":2000}]}));
    h.send(hand_start("h1"));
    h.send(json!({"type":"hole_cards","cards":["Kh","Kd"]}));
    h.send(pa(4, "raise", Some(60), "preflop", 1940, 90));
    h.send(pa(0, "fold", None, "preflop", 2000, 90));
    h.send(pa(1, "fold", None, "preflop", 1990, 90));
    let pl = json!([
        {"seat":0,"stack":2000},{"seat":1,"stack":1990},{"seat":2,"stack":1980},{"seat":4,"stack":1940}
    ]);
    let out = h.send(your_turn(
        "h1",
        "tk",
        json!([{"action":"fold"},{"action":"call","amount":40},{"action":"raise","min":100,"max":2000},{"action":"all_in","amount":2000}]),
        90,
        &[],
        pl,
    ));
    assert_eq!(out.len(), 1);
    assert!(h.bad_anomalies().is_empty(), "{:#?}", h.bad_anomalies());
    let d = &h.sink.of_kind("decision")[0];
    assert_eq!(d["meta"]["n_players"], 4);
    assert!(d["fallback"].is_null());
    assert!(!d["dist"].as_array().unwrap().is_empty());
}

// ------------------------------------------------------------------ stack limit (reset + guard)

fn empty_hand(h: &mut H, id: &str, final_stack: i64) -> Vec<Value> {
    h.send(hand_start(id));
    h.send(json!({"type":"hole_cards","cards":["7h","2d"]}));
    h.send(json!({"type":"hand_result","hand_id":id,"winners":[],"final_stacks":{"2":final_stack}}))
}

fn count(out: &[Value], ty: &str) -> usize {
    out.iter().filter(|m| m["type"] == ty).count()
}

fn own_left() -> Value {
    json!({"type":"player_left","seat":2,"name":"me","reason":"left"})
}

#[test]
fn over_deep_stack_leaves_once_rejoins_with_buy_in_and_resumes() {
    let (sp, _) = spec(false, false);
    let mut h = H::with_cap(sp, 100, 150.0);
    h.send(json!({"type":"connected","agent_id":"a","name":"me"}));
    h.send(table_joined());
    // 1,000 bb... well over 150 bb (cap = 3000 chips): leave, exactly once.
    let out = empty_hand(&mut h, "big1", 22_000);
    assert_eq!(count(&out, "leave_table"), 1, "{out:?}");
    // Still waiting for the server: another hand result must not send a second leave.
    let out = empty_hand(&mut h, "big2", 22_000);
    assert_eq!(count(&out, "leave_table"), 0);
    // Server confirms our departure -> join the lobby with the normal buy-in.
    let out = h.send(own_left());
    assert_eq!(count(&out, "join_lobby"), 1);
    let jl = out.iter().find(|m| m["type"] == "join_lobby").unwrap();
    assert_eq!(jl["buy_in"], 2000);
    assert_eq!(h.s.counters.stack_resets, 1);
    // Another player leaving is not us.
    assert!(h.send(json!({"type":"player_left","seat":4,"name":"bot4","reason":"left"})).is_empty());
    h.send(json!({"type":"lobby_joined","position":1}));
    // New table, fresh sequence numbers; play resumes and a normal-depth hand does not reset.
    h.send(table_joined());
    play_canonical_hand(&mut h, "after");
    assert!(h.bad_anomalies().is_empty(), "{:#?}", h.bad_anomalies());
    assert_eq!(h.s.counters.stack_resets, 1);
    assert_eq!(h.s.counters.hands_done, 3);
    assert!(h.sink.of_kind("note").iter().any(|n| n["what"] == "stack_reset_leave"));
    assert!(h.s.on_idle().is_empty());
}

#[test]
fn refused_leave_falls_back_to_the_guard_with_cooldown() {
    let (sp, _) = spec(false, false);
    let mut h = H::with_cap(sp, 100, 150.0);
    h.send(json!({"type":"connected","agent_id":"a","name":"me"}));
    h.send(table_joined());
    assert_eq!(count(&empty_hand(&mut h, "b1", 22_000), "leave_table"), 1);
    let out = h.send(json!({"type":"error","code":"cooldown","message":"nope"}));
    assert!(out.is_empty());
    assert_eq!(h.s.counters.reset_aborts, 1);
    assert_eq!(h.s.counters.stack_resets, 0);
    // Cooldown: no leave/rejoin loop on the next over-deep hands.
    assert_eq!(count(&empty_hand(&mut h, "b2", 22_000), "leave_table"), 0);
    assert!(h.sink.of_kind("note").iter().any(|n| n["what"] == "stack_reset_aborted"));
}

#[test]
fn stalled_rejoin_is_retried_then_given_up() {
    let (sp, _) = spec(false, false);
    let mut h = H::with_cap(sp, 100, 150.0);
    h.send(json!({"type":"connected","agent_id":"a","name":"me"}));
    h.send(table_joined());
    empty_hand(&mut h, "b1", 22_000);
    h.send(own_left());
    let t0 = std::time::Instant::now();
    assert!(h.s.on_idle_at(t0).is_empty(), "too early");
    let out = h.s.on_idle_at(t0 + std::time::Duration::from_secs(16));
    assert_eq!(count(&out, "join_lobby"), 1);
    // table_joined ends the rejoin; no more retries.
    h.send(table_joined());
    assert!(h.s.on_idle_at(t0 + std::time::Duration::from_secs(100)).is_empty());
}

/// Hero (BB, seat 2) at 1,000 bb faces a 3x open; the mock policy shoves.
fn deep_hand(h: &mut H, id: &str) -> Vec<Value> {
    h.send(hand_start(id));
    h.send(json!({"type":"hole_cards","cards":["Ah","Ad"]}));
    h.send(pa(3, "fold", None, "preflop", 20_000, 30));
    h.send(pa(4, "raise", Some(60), "preflop", 19_940, 90));
    h.send(pa(5, "fold", None, "preflop", 20_000, 90));
    h.send(pa(0, "fold", None, "preflop", 20_000, 90));
    h.send(pa(1, "fold", None, "preflop", 19_990, 90));
    let pl = players([20_000, 19_990, 19_980, 20_000, 19_940, 20_000], ["folded", "folded", "active", "folded", "active", "folded"]);
    h.send(your_turn(
        id,
        "tk",
        json!([{"action":"fold"},{"action":"call","amount":40},{"action":"raise","min":100,"max":20000}]),
        90,
        &[],
        pl,
    ))
}

#[test]
fn deep_stack_jam_is_capped_at_max_stack_bb() {
    let (sp, _) = spec(false, true);
    let mut h = H::with_cap(sp, 100, 150.0);
    h.send(table_joined());
    let out = deep_hand(&mut h, "deep");
    assert_eq!(out[0]["action"], "raise");
    assert_eq!(out[0]["amount"], 3000, "150 bb x 20 = 3000 chips total, not the 20,000 jam");
    let d = &h.sink.of_kind("decision")[0];
    assert!(d["stack_guard"].as_str().unwrap().contains("raise:3000"), "{d}");
    assert_eq!(h.s.counters.stack_guards, 1);
    // The policy observes what was actually sent: echo + villain fold replay cleanly.
    h.send(pa(2, "raise", Some(3000), "preflop", 17_000, 3070));
    h.send(pa(4, "fold", None, "preflop", 19_940, 3070));
    h.send(json!({"type":"hand_result","hand_id":"deep","winners":[{"seat":2,"amount":60}],"payouts":[{"seat":2,"amount":3090}],
        "final_stacks":{"2":20_060,"4":19_940,"0":20000,"1":19990,"3":20000,"5":20000}}));
    assert!(h.bad_anomalies().is_empty(), "{:#?}", h.bad_anomalies());
    let r = &h.hands()[0];
    assert!(!r.desync && r.issues == 0);
}

#[test]
fn guard_converts_to_call_when_cap_is_below_min_raise_and_keeps_calls_and_folds() {
    use openpoker::mapping::{cap_action, OpAction};
    use openpoker::proto::ValidSet;
    let v = ValidSet { fold: true, check: false, call: Some(500), raise: Some((1000, 20_000)), all_in: None };
    let shove = OpAction { action: "raise".into(), amount: Some(20_000) };
    // cap leaves less than the min raise -> call
    let (a, _) = cap_action(&shove, &v, 800, 0, 20_000).unwrap();
    assert_eq!(a.action, "call");
    // call / fold are never touched, even beyond the cap
    assert!(cap_action(&OpAction { action: "call".into(), amount: None }, &v, 100, 0, 20_000).is_none());
    assert!(cap_action(&OpAction { action: "fold".into(), amount: None }, &v, 100, 0, 20_000).is_none());
    // all-in that fits under the cap is allowed; one that does not becomes a raise to the cap
    let ai = OpAction { action: "all_in".into(), amount: None };
    assert!(cap_action(&ai, &v, 3000, 200, 2500).is_none());
    let (a, _) = cap_action(&ai, &v, 3000, 200, 20_000).unwrap();
    assert_eq!((a.action.as_str(), a.amount), ("raise", Some(2800)));
}

#[test]
fn max_stack_bb_zero_changes_nothing() {
    let (sp, _) = spec(false, true);
    let mut h = H::with_cap(sp, 100, 0.0);
    h.send(table_joined());
    let out = deep_hand(&mut h, "deep");
    assert_eq!(out[0]["amount"], 20_000, "full jam when disabled");
    assert!(h.sink.of_kind("decision")[0]["stack_guard"].is_null());
    h.send(pa(2, "raise", Some(20_000), "preflop", 0, 20_090));
    h.send(pa(4, "fold", None, "preflop", 19_940, 20_090));
    let out = h.send(json!({"type":"hand_result","hand_id":"deep","winners":[],"final_stacks":{"2":40_000}}));
    assert_eq!(count(&out, "leave_table"), 0, "no reset when disabled");
    assert_eq!((h.s.counters.stack_guards, h.s.counters.stack_resets), (0, 0));
}
