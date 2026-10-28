//! Conformance tests for TABLE_API.md section 12, plus auth, error paths, house bots,
//! duplicate matches, hand history export and the leaderboard. Each test spawns the server
//! in-process on an ephemeral port with an in-memory SQLite database and drives it over
//! real HTTP and WebSocket connections.

use arena_server::config::ServerConfig;
use arena_server::{start, RunningServer};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::{connect_async, tungstenite::Message};

struct Srv {
    base: String,
    ws: String,
    rs: RunningServer,
    http: reqwest::Client,
}

/// A tiny compact blueprint for the sharingan-1 house bot, if one is available on this machine.
fn test_blueprint() -> Option<std::path::PathBuf> {
    let p = std::env::var("SHARINGAN_TEST_BLUEPRINT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("dev-serve/tinybp/tiny.plcp"));
    p.exists().then_some(p)
}

async fn srv() -> Srv {
    srv_with(None).await
}

async fn srv_with(blueprint: Option<std::path::PathBuf>) -> Srv {
    let cfg = ServerConfig {
        blueprint,
        sizing: "tiny".into(),
        bind: "127.0.0.1:0".into(),
        db_path: ":memory:".into(),
        admin_token: Some("admin".into()),
        hand_delay_ms: 0,
        reconnect_grace_ms: 200,
        min_leaderboard_hands: 6,
        ..Default::default()
    };
    let rs = start(cfg).await.unwrap();
    Srv { base: format!("http://{}", rs.addr), ws: format!("ws://{}", rs.addr), rs, http: reqwest::Client::new() }
}

impl Srv {
    async fn req(&self, method: &str, path: &str, token: Option<&str>, body: Option<Value>) -> (u16, Value) {
        let m = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
        let mut r = self.http.request(m, format!("{}{}", self.base, path));
        if let Some(t) = token {
            r = r.bearer_auth(t);
        }
        if let Some(b) = body {
            r = r.json(&b);
        }
        let resp = r.send().await.unwrap();
        let st = resp.status().as_u16();
        let txt = resp.text().await.unwrap();
        (st, serde_json::from_str(&txt).unwrap_or(Value::String(txt)))
    }
    async fn post(&self, p: &str, t: Option<&str>, b: Value) -> (u16, Value) {
        self.req("POST", p, t, Some(b)).await
    }
    async fn get(&self, p: &str, t: Option<&str>) -> (u16, Value) {
        self.req("GET", p, t, None).await
    }
    async fn register(&self, name: &str) -> (String, String) {
        let (st, v) = self.post("/v1/bots", None, json!({"name": name, "kind": "bot"})).await;
        assert_eq!(st, 201, "{v}");
        (v["player_id"].as_str().unwrap().into(), v["token"].as_str().unwrap().into())
    }
    async fn table(&self, cfg: Value) -> String {
        let (st, v) = self.post("/v1/tables", Some("admin"), cfg).await;
        assert_eq!(st, 201, "{v}");
        v["table_id"].as_str().unwrap().into()
    }
    async fn wait_match(&self, id: &str) -> Value {
        for _ in 0..600 {
            let (_, v) = self.get(&format!("/v1/matches/{id}"), None).await;
            if v["status"] == "finished" || v["status"] == "aborted" {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("match did not finish");
    }
}

enum Out {
    Text(String),
    Close,
}

struct Room {
    ws_base: String,
    ids: Vec<(String, String)>,
    outs: Vec<Option<UnboundedSender<Out>>>,
    rx: UnboundedReceiver<(usize, Value)>,
    mtx: UnboundedSender<(usize, Value)>,
}

type Act = Option<(&'static str, Option<i64>)>;

impl Room {
    async fn new(s: &Srv, n: usize) -> Room {
        let (mtx, rx) = unbounded_channel();
        let mut r = Room { ws_base: s.ws.clone(), ids: vec![], outs: vec![], rx, mtx };
        for i in 0..n {
            let id = s.register(&format!("bot{i}")).await;
            r.ids.push(id);
            r.outs.push(None);
            r.connect(i, true).await;
        }
        r
    }
    async fn connect(&mut self, i: usize, wait_welcome: bool) {
        let url = format!("{}/v1/play?token={}", self.ws_base, self.ids[i].1);
        let (ws, _) = connect_async(url).await.unwrap();
        let (mut w, mut rd) = ws.split();
        let (otx, mut orx) = unbounded_channel::<Out>();
        tokio::spawn(async move {
            while let Some(o) = orx.recv().await {
                match o {
                    Out::Text(s) => {
                        if w.send(Message::Text(s.into())).await.is_err() {
                            break;
                        }
                    }
                    Out::Close => {
                        let _ = w.close().await;
                        break;
                    }
                }
            }
        });
        let mtx = self.mtx.clone();
        tokio::spawn(async move {
            while let Some(Ok(m)) = rd.next().await {
                if let Message::Text(t) = m {
                    let v: Value = serde_json::from_str(t.as_str()).unwrap();
                    if mtx.send((i, v)).is_err() {
                        break;
                    }
                }
            }
        });
        self.outs[i] = Some(otx);
        self.send(i, "hello", None, json!({"client": "test/0.1", "resume": []}));
        if wait_welcome {
            loop {
                let (j, m) = self.recv().await;
                if j == i && m["type"] == "welcome" {
                    break;
                }
            }
        }
    }
    fn send(&self, i: usize, ty: &str, table: Option<&str>, data: Value) {
        let mut m = json!({"v": 1, "type": ty, "t_ms": 0, "data": data});
        if let Some(t) = table {
            m["table_id"] = json!(t);
        }
        let _ = self.outs[i].as_ref().unwrap().send(Out::Text(m.to_string()));
    }
    fn close(&mut self, i: usize) {
        let _ = self.outs[i].take().unwrap().send(Out::Close);
    }
    async fn recv(&mut self) -> (usize, Value) {
        tokio::time::timeout(Duration::from_secs(20), self.rx.recv()).await.expect("timed out waiting for a message").unwrap()
    }
    async fn sit_all(&self, s: &Srv, table: &str, seats: &[usize]) {
        for (i, seat) in seats.iter().enumerate() {
            let (st, v) = s
                .post(&format!("/v1/tables/{table}/seats/{seat}"), Some(&self.ids[i].1), json!({"player_id": self.ids[i].0}))
                .await;
            assert_eq!(st, 200, "{v}");
        }
    }
    fn act(&self, i: usize, table: &str, d: &Value, ty: &str, to: Option<i64>) {
        let mut data = json!({"hand_id": d["hand_id"], "decision_id": d["decision_id"], "type": ty});
        if let Some(t) = to {
            data["to"] = json!(t);
        }
        self.send(i, "act", Some(table), data);
    }
    /// Play one hand: answer every `your_turn` with `policy(client, your_turn.data)`; `None` = stay silent.
    async fn run_hand(&mut self, table: &str, listeners: usize, mut policy: impl FnMut(usize, &Value) -> Act) -> Vec<(usize, Value)> {
        let mut all = Vec::new();
        let mut ends = 0;
        while ends < listeners {
            let (i, m) = self.recv().await;
            match m["type"].as_str().unwrap() {
                "your_turn" => {
                    let d = m["data"].clone();
                    if let Some((ty, to)) = policy(i, &d) {
                        self.act(i, table, &d, ty, to);
                    }
                }
                "hand_end" => ends += 1,
                _ => {}
            }
            all.push((i, m));
        }
        all
    }
}

fn of_type<'a>(evs: &'a [(usize, Value)], ty: &'a str) -> impl Iterator<Item = &'a Value> + 'a {
    evs.iter().filter(move |(_, m)| m["type"] == ty).map(|(_, m)| m)
}

/// Raise to max if possible, else call, else check.
fn jam(d: &Value) -> Act {
    let l = &d["legal"];
    if !l["raise"].is_null() {
        Some(("raise", l["raise"]["max_to"].as_i64()))
    } else if !l["call"].is_null() {
        Some(("call", None))
    } else {
        Some(("check", None))
    }
}

fn fold_or_check(d: &Value) -> Act {
    Some(if d["legal"]["check"] == true { ("check", None) } else { ("fold", None) })
}

// ---- 12.1 blinds and first action ----

#[tokio::test]
async fn v1_blinds_and_first_action() {
    let s = srv().await;
    let t = s.table(json!({"mode": "sandbox", "sandbox": {"button_seat": 3}, "decision_time_ms": 5000})).await;
    let mut room = Room::new(&s, 6).await;
    room.sit_all(&s, &t, &[0, 1, 2, 3, 4, 5]).await;
    let mut first = None;
    let evs = room
        .run_hand(&t, 6, |_, d| {
            first.get_or_insert(d["seat"].as_u64().unwrap());
            Some(("fold", None))
        })
        .await;
    assert_eq!(first, Some(0));
    let hs = of_type(&evs, "hand_start").next().unwrap();
    assert_eq!(hs["data"]["button_seat"], 3);
    assert_eq!(hs["data"]["sb_seat"], 4);
    assert_eq!(hs["data"]["bb_seat"], 5);
    let acts: Vec<&Value> = of_type(&evs, "action").filter(|m| m["data"]["seat"].is_u64()).collect();
    let first_actions: Vec<(String, u64, i64)> = acts
        .iter()
        .take(2)
        .map(|m| (m["data"]["type"].as_str().unwrap().into(), m["data"]["seat"].as_u64().unwrap(), m["data"]["to"].as_i64().unwrap()))
        .collect();
    // every client sees every action; the first two distinct events are the blinds
    assert_eq!(first_actions[0], ("post_sb".to_string(), 4, 50));
    assert_eq!(first_actions[1], ("post_bb".to_string(), 5, 100));
    assert_eq!(acts[0]["data"]["seq"], 1);
    assert_eq!(acts[0]["data"]["forced"], true);
}

#[tokio::test]
async fn v1_heads_up_button_is_sb_and_acts_first() {
    let s = srv().await;
    let t = s.table(json!({"mode": "sandbox", "min_players_to_deal": 2, "sandbox": {"button_seat": 4}})).await;
    let mut room = Room::new(&s, 2).await;
    room.sit_all(&s, &t, &[1, 4]).await;
    let mut first = None;
    let evs = room
        .run_hand(&t, 2, |_, d| {
            first.get_or_insert(d["seat"].as_u64().unwrap());
            Some(("fold", None))
        })
        .await;
    assert_eq!(first, Some(4));
    let hs = of_type(&evs, "hand_start").next().unwrap();
    assert_eq!(hs["data"]["sb_seat"], 4);
    assert_eq!(hs["data"]["bb_seat"], 1);
}

// ---- 12.2 min-raise chain ----

#[tokio::test]
async fn v2_min_raise_chain() {
    let s = srv().await;
    let t = s.table(json!({"mode": "sandbox", "sandbox": {"button_seat": 3}})).await;
    let mut room = Room::new(&s, 6).await;
    room.sit_all(&s, &t, &[0, 1, 2, 3, 4, 5]).await;
    let mut mins: Vec<(u64, i64)> = vec![];
    room.run_hand(&t, 6, |_, d| {
        let seat = d["seat"].as_u64().unwrap();
        let min = d["legal"]["raise"]["min_to"].as_i64().unwrap_or(-1);
        mins.push((seat, min));
        match (mins.len(), seat) {
            (1, 0) => Some(("raise", Some(300))),
            (2, 1) => Some(("raise", Some(500))),
            (3, 2) => Some(("raise", Some(1200))),
            _ => Some(("fold", None)),
        }
    })
    .await;
    assert_eq!(&mins[..4], &[(0, 200), (1, 500), (2, 700), (3, 1900)]);
}

// ---- 12.3 short all-in does not reopen ----

#[tokio::test]
async fn v3_short_all_in_does_not_reopen() {
    let s = srv().await;
    let t = s.table(json!({"mode": "sandbox", "starting_stack": 1500, "sandbox": {"button_seat": 3}})).await;
    let mut room = Room::new(&s, 6).await;
    room.sit_all(&s, &t, &[0, 1, 2, 3, 4, 5]).await;
    let mut flop: Vec<(u64, Value)> = vec![];
    let evs = room
        .run_hand(&t, 6, |_, d| {
            let seat = d["seat"].as_u64().unwrap();
            if d["street"] == "preflop" {
                return match seat {
                    4 => Some(("call", None)),
                    5 => Some(("check", None)),
                    _ => Some(("fold", None)),
                };
            }
            flop.push((seat, d["legal"].clone()));
            match flop.len() {
                1 => Some(("bet", Some(1000))),     // seat 4 bets 1000
                2 => Some(("raise", Some(1400))),   // seat 5 all-in for less than a full raise
                _ => Some(("call", None)),          // seat 4: only call/fold
            }
        })
        .await;
    assert_eq!(flop.len(), 3);
    // BB facing the bet: min raise is capped at the all-in amount
    assert_eq!(flop[1].1["raise"], json!({"min_to": 1400, "max_to": 1400}));
    // the bettor may only call or fold now
    assert!(flop[2].1["raise"].is_null());
    assert_eq!(flop[2].1["call"]["to"], 1400);
    let end = of_type(&evs, "hand_end").next().unwrap();
    let net: i64 = end["data"]["results"].as_array().unwrap().iter().map(|r| r["net"].as_i64().unwrap()).sum();
    assert_eq!(net, 0);
}

// ---- 12.4 side pots ----

#[tokio::test]
async fn v4_side_pots() {
    let s = srv().await;
    let deck: Vec<&str> = vec!["As", "Ks", "7c", "Ah", "Kh", "2d", "3c", "4d", "5h", "9s", "3d", "Jc", "3h", "Qd"];
    let t = s
        .table(json!({"mode": "sandbox", "min_players_to_deal": 3,
                      "sandbox": {"button_seat": 2, "stacks": [2000, 5000, 10000], "decks": [deck]}}))
        .await;
    let mut room = Room::new(&s, 3).await;
    room.sit_all(&s, &t, &[0, 1, 2]).await;
    let evs = room.run_hand(&t, 3, |_, d| jam(d)).await;
    let end = of_type(&evs, "hand_end").next().unwrap()["data"].clone();
    let pots = end["pots"].as_array().unwrap();
    let amounts: Vec<i64> = pots.iter().map(|p| p["amount"].as_i64().unwrap()).collect();
    assert_eq!(amounts, vec![6000, 6000, 5000]);
    assert_eq!(pots[0]["eligible_seats"], json!([0, 1, 2]));
    assert_eq!(pots[1]["eligible_seats"], json!([1, 2]));
    assert_eq!(pots[2]["eligible_seats"], json!([2]));
    assert_eq!(pots[0]["winners"], json!([0]));
    assert_eq!(pots[1]["winners"], json!([1]));
    let nets: Vec<i64> = end["results"].as_array().unwrap().iter().map(|r| r["net"].as_i64().unwrap()).collect();
    assert_eq!(nets, vec![4000, 1000, -5000]);
    assert_eq!(end["ended_by"], "showdown");
    assert_eq!(end["hole_cards"]["0"], json!(["As", "Ah"]));
}

// ---- 12.5 odd chip ----

#[tokio::test]
async fn v5_odd_chip_goes_left_of_button() {
    let s = srv().await;
    let deck: Vec<&str> = vec!["4c", "Ac", "As", "5d", "Kd", "Kh", "6s", "2h", "3h", "7d", "6d", "9c", "6h", "Jd"];
    let t = s
        .table(json!({"mode": "sandbox", "min_players_to_deal": 3, "small_blind": 1, "big_blind": 2, "starting_stack": 500,
                      "sandbox": {"button_seat": 5, "decks": [deck]}}))
        .await;
    let mut room = Room::new(&s, 3).await;
    room.sit_all(&s, &t, &[1, 3, 5]).await;
    let evs = room
        .run_hand(&t, 3, |_, d| if d["seat"] == 1 { Some(("fold", None)) } else { jam(d) })
        .await;
    let end = of_type(&evs, "hand_end").next().unwrap()["data"].clone();
    let pot = &end["pots"][0];
    assert_eq!(pot["amount"], 1001);
    assert_eq!(pot["winners"], json!([3, 5]));
    assert_eq!(pot["share"], json!([501, 500]));
}

// ---- 12.6 timeout ----

#[tokio::test]
async fn v6_timeout_folds_when_facing_a_bet_and_checks_when_free() {
    let s = srv().await;
    let t = s.table(json!({"mode": "sandbox", "decision_time_ms": 300, "sandbox": {"button_seat": 3}})).await;
    let mut room = Room::new(&s, 6).await;
    room.sit_all(&s, &t, &[0, 1, 2, 3, 4, 5]).await;
    let evs = room.run_hand(&t, 6, |_, _| None).await;
    let a = of_type(&evs, "action").find(|m| m["data"]["seat"] == 0).unwrap();
    assert_eq!(a["data"]["type"], "fold");
    assert_eq!(a["data"]["timeout"], true);

    // heads-up: the BB is silent when checking is free -> check with timeout:true
    let s2 = srv().await;
    let t2 = s2.table(json!({"mode": "sandbox", "min_players_to_deal": 2, "decision_time_ms": 300, "sandbox": {"button_seat": 0}})).await;
    let mut room2 = Room::new(&s2, 2).await;
    room2.sit_all(&s2, &t2, &[0, 1]).await;
    let evs = room2
        .run_hand(&t2, 2, |_, d| if d["seat"] == 0 { fold_or_check(d).map(|_| ("call", None)).filter(|_| d["legal"]["call"] != Value::Null).or(Some(("check", None))) } else { None })
        .await;
    let bb_checks: Vec<&Value> = of_type(&evs, "action")
        .filter(|m| m["data"]["seat"] == 1 && m["data"]["type"] == "check" && m["data"]["timeout"] == true)
        .collect();
    assert!(!bb_checks.is_empty(), "BB should have been checked by the server");
}

// ---- 12.7 reconnect mid-hand ----

#[tokio::test]
async fn v7_reconnect_resumes_same_decision() {
    let s = srv().await;
    let t = s.table(json!({"mode": "sandbox", "decision_time_ms": 4000, "sandbox": {"button_seat": 3}})).await;
    let mut room = Room::new(&s, 6).await;
    room.sit_all(&s, &t, &[0, 1, 2, 3, 4, 5]).await;
    let (turn, i) = loop {
        let (i, m) = room.recv().await;
        if m["type"] == "your_turn" {
            break (m, i);
        }
    };
    assert_eq!(i, 0);
    let decision = turn["data"]["decision_id"].as_str().unwrap().to_string();
    room.close(0);
    tokio::time::sleep(Duration::from_millis(100)).await;
    room.connect(0, false).await;
    room.send(0, "hello", None, json!({"client": "t", "resume": [{"table_id": t, "last_table_seq": 0}]}));
    let (mut welcome, mut snap, mut again) = (false, false, None);
    while again.is_none() {
        let (i, m) = room.recv().await;
        if i != 0 {
            continue;
        }
        match m["type"].as_str().unwrap() {
            "welcome" => welcome = true,
            "table_snapshot" => {
                snap = true;
                assert!(m["data"]["hand"]["hand_id"].is_string());
                assert_eq!(m["data"]["hole_cards"]["0"].as_array().unwrap().len(), 2);
            }
            "your_turn" => again = Some(m),
            _ => {}
        }
    }
    assert!(welcome && snap);
    let again = again.unwrap();
    assert_eq!(again["data"]["decision_id"].as_str().unwrap(), decision);
    room.act(0, &t, &again["data"], "fold", None);
    loop {
        let (_, m) = room.recv().await;
        if m["type"] == "action" && m["data"]["seat"] == 0 && m["data"]["type"] == "fold" {
            assert_eq!(m["data"]["timeout"], false);
            break;
        }
    }
}

// ---- errors: illegal action, bad amount, idempotency, stale ----

#[tokio::test]
async fn illegal_actions_get_errors_and_a_fresh_your_turn() {
    let s = srv().await;
    let t = s.table(json!({"mode": "sandbox", "min_players_to_deal": 2, "sandbox": {"button_seat": 0}})).await;
    let mut room = Room::new(&s, 2).await;
    room.sit_all(&s, &t, &[0, 1]).await;
    let turn = loop {
        let (_, m) = room.recv().await;
        if m["type"] == "your_turn" {
            break m;
        }
    };
    let d = turn["data"].clone();
    room.act(0, &t, &d, "raise", Some(150)); // below min
    room.act(0, &t, &d, "check", None); // there is a bet
    let mut errs = vec![];
    let mut turns = 0;
    while errs.len() < 2 || turns < 2 {
        let (i, m) = room.recv().await;
        if i != 0 {
            continue;
        }
        match m["type"].as_str().unwrap() {
            "error" => errs.push(m),
            "your_turn" => {
                turns += 1;
                assert_eq!(m["data"]["decision_id"], d["decision_id"]);
                assert_eq!(m["table_seq"], turn["table_seq"], "a re-sent your_turn repeats its table_seq");
            }
            _ => {}
        }
    }
    assert_eq!(errs[0]["data"]["code"], "bad_amount");
    assert_eq!(errs[0]["data"]["details"]["min_to"], 200);
    assert_eq!(errs[1]["data"]["code"], "illegal_action");
    // valid call, twice (idempotent), plus an unknown decision id
    room.act(0, &t, &d, "call", None);
    room.act(0, &t, &d, "call", None);
    room.send(0, "act", Some(&t), json!({"hand_id": d["hand_id"], "decision_id": "d-bogus", "type": "fold"}));
    let mut got_stale = false;
    let mut seen_call = 0;
    let mut bb_turn = None;
    while bb_turn.is_none() || seen_call == 0 {
        let (i, m) = room.recv().await;
        match (i, m["type"].as_str().unwrap()) {
            (0, "error") => {
                assert_eq!(m["data"]["code"], "stale_decision");
                got_stale = true;
            }
            (_, "action") if m["data"]["seat"] == 0 && m["data"]["type"] == "call" && i == 0 => seen_call += 1,
            (1, "your_turn") => bb_turn = Some(m),
            _ => {}
        }
    }
    assert_eq!(seen_call, 1);
    // BB folds with check free: converted to a check, no error
    let bt = bb_turn.unwrap();
    assert_eq!(bt["data"]["legal"]["check"], true);
    room.act(1, &t, &bt["data"], "fold", None);
    loop {
        let (i, m) = room.recv().await;
        if i == 1 && m["type"] == "action" && m["data"]["seat"] == 1 {
            assert_eq!(m["data"]["type"], "check");
            break;
        }
        if i == 1 && m["type"] == "error" {
            panic!("fold-when-free must not error: {m}");
        }
    }
    let _ = got_stale;
}

// ---- auth, hashing ----

#[tokio::test]
async fn auth_and_token_hashing() {
    let s = srv().await;
    assert_eq!(s.get("/v1/hands/nope", None).await.0, 401);
    assert_eq!(s.post("/v1/tables", None, json!({})).await.0, 401);
    assert_eq!(s.post("/v1/tables", Some("wrong"), json!({})).await.0, 401);
    let (pid, tok) = s.register("alice").await;
    let rec = s.rs.app.store.get_player(&pid).unwrap().unwrap();
    assert_ne!(rec.token_hash, tok);
    assert_eq!(rec.token_hash.len(), 64);
    let (st, v) = s.get(&format!("/v1/players/{pid}"), None).await;
    assert_eq!(st, 200);
    assert!(v.get("token").is_none() && v.get("token_hash").is_none());
    // a bot token works as bearer; a bad ws token is refused
    assert_eq!(s.post("/v1/tables", Some(&tok), json!({"decision_time_ms": 1000})).await.0, 201);
    assert!(connect_async(format!("{}/v1/play?token=bad", s.ws)).await.is_err());
    // unknown house bot gives a clear error
    let (st, v) = s.post("/v1/matches", Some("admin"), json!({"participants": [pid.clone(), "house:sharingan-1"], "hands_planned": 6})).await;
    assert_eq!(st, 400, "{v}");
}

// ---- external bot vs house bots on a free table ----

#[tokio::test]
async fn single_bot_plays_a_full_table_against_house_bots() {
    let s = srv().await;
    let t = s.table(json!({"decision_time_ms": 3000})).await;
    let mut room = Room::new(&s, 1).await;
    room.sit_all(&s, &t, &[0]).await;
    let (st, v) = s.post(&format!("/v1/tables/{t}/fill"), Some("admin"), json!({"bot": "ladder"})).await;
    assert_eq!(st, 400, "ladder needs sharingan-1: {v}");
    let (st, v) = s.post(&format!("/v1/tables/{t}/fill"), Some("admin"), json!({"bot": "house:tight_aggressive"})).await;
    assert_eq!(st, 200, "{v}");
    for _ in 0..5 {
        let evs = room.run_hand(&t, 1, |_, d| if d["legal"]["check"] == true { Some(("check", None)) } else { Some(("call", None)) }).await;
        let end = of_type(&evs, "hand_end").next().unwrap();
        let rs = end["data"]["results"].as_array().unwrap();
        assert_eq!(rs.len(), 6);
        assert_eq!(rs.iter().map(|r| r["net"].as_i64().unwrap()).sum::<i64>(), 0);
        for r in rs {
            assert_eq!(r["stack_end"].as_i64().unwrap(), 10000 + r["net"].as_i64().unwrap());
        }
        // seq is gap-free over the sequenced events seen by this client
        let seqs: Vec<i64> = evs.iter().filter(|(_, m)| m["seq"].is_u64() && m["type"] != "your_turn").map(|(_, m)| m["seq"].as_i64().unwrap()).collect();
        assert!(seqs.windows(2).all(|w| w[1] > w[0]));
    }
    let (_, tj) = s.get(&format!("/v1/tables/{t}"), None).await;
    assert_eq!(tj["seats"].as_array().unwrap().len(), 6);
}

// ---- 12.8 / 12.9 duplicate matches, history, standings, leaderboard ----

fn participants() -> Value {
    json!(["house:random", "house:calling_station", "house:tight_aggressive", "house:nit", "house:lag", "house:maniac"])
}

#[tokio::test]
async fn v8_v9_duplicate_matches_are_reproducible_and_consistent() {
    let s = srv().await;
    let mk = |seed: &str| json!({"participants": participants(), "hands_planned": 24, "duplicate": {"enabled": true, "rotations": 6, "deck_from": "seed"},
                                 "seed": seed, "block_deals": 2, "table_config": {"decision_time_ms": 1000}});
    let mut ids = vec![];
    for seed in ["abc", "abc", "xyz"] {
        let (st, v) = s.post("/v1/matches", Some("admin"), mk(seed)).await;
        assert_eq!(st, 201, "{v}");
        ids.push(v["match_id"].as_str().unwrap().to_string());
    }
    for id in &ids {
        let (st, _) = s.post(&format!("/v1/matches/{id}/start"), Some("admin"), json!({})).await;
        assert_eq!(st, 200);
    }
    let mut hands = vec![];
    for id in &ids {
        let m = s.wait_match(id).await;
        assert_eq!(m["status"], "finished", "{m}");
        assert_eq!(m["hands_played"], 24);
        let (st, h) = s.get(&format!("/v1/matches/{id}/hands?limit=500"), Some("admin")).await;
        assert_eq!(st, 200);
        let h: Vec<Value> = h.as_array().unwrap().clone();
        assert_eq!(h.len(), 24);
        hands.push((m, h));
    }
    // same seed -> identical decks hand by hand; a different seed differs
    for k in 0..24 {
        assert_eq!(hands[0].1[k]["deck"], hands[1].1[k]["deck"]);
        assert_eq!(hands[0].1[k]["deal_no"], hands[1].1[k]["deal_no"]);
    }
    assert_ne!(hands[0].1[0]["deck"], hands[2].1[0]["deck"]);
    // rotation r puts participant p in seat (p + r) mod 6; the cards fall on the same seats
    let (m, h) = &hands[0];
    let parts: Vec<String> = m["participants"].as_array().unwrap().iter().map(|p| p.as_str().unwrap().to_string()).collect();
    let mut by_deal: std::collections::BTreeMap<u64, Vec<&Value>> = Default::default();
    for x in h {
        by_deal.entry(x["deal_no"].as_u64().unwrap()).or_default().push(x);
        let r = x["rotation"].as_u64().unwrap() as usize;
        for p in x["players"].as_array().unwrap() {
            let seat = p["seat"].as_u64().unwrap() as usize;
            let idx = parts.iter().position(|q| q == p["player_id"].as_str().unwrap()).unwrap();
            assert_eq!((idx + r) % 6, seat);
        }
        // v9
        let net: i64 = x["results"].as_array().unwrap().iter().map(|r| r["net"].as_i64().unwrap()).sum();
        assert_eq!(net, 0);
        for (r, p) in x["results"].as_array().unwrap().iter().zip(x["players"].as_array().unwrap()) {
            assert_eq!(r["stack_end"].as_i64().unwrap(), p["stack_start"].as_i64().unwrap() + r["net"].as_i64().unwrap());
        }
        assert_eq!(x["anomalies"], json!([]));
    }
    assert_eq!(by_deal.len(), 4);
    for (_, hs) in by_deal {
        assert_eq!(hs.len(), 6);
        let mut rots: Vec<u64> = hs.iter().map(|x| x["rotation"].as_u64().unwrap()).collect();
        rots.sort();
        assert_eq!(rots, vec![0, 1, 2, 3, 4, 5]);
        assert!(hs.iter().all(|x| x["hole_cards"] == hs[0]["hole_cards"] && x["button_seat"] == hs[0]["button_seat"]));
    }
    // standings: a list with the SDK-facing fields; chips are conserved
    let standings = m["standings"].as_array().unwrap();
    assert_eq!(standings.len(), 6);
    let mut total = 0;
    for st in standings {
        assert!(st["player_id"].is_string() && st["mbb_per_game"].is_number() && st["ci95_mbb"].is_number());
        assert!(st["bb_per_100"].is_number() && st["ci95_bb_per_100"].is_number() && st["duplicate_bb_per_100"].is_number());
        assert_eq!(st["method"], "aivat");
        total += st["net_chips"].as_i64().unwrap();
    }
    assert_eq!(total, 0);
    // jsonl export
    let r = s.http.get(format!("{}/v1/matches/{}/hands.jsonl", s.base, ids[0])).bearer_auth("admin").send().await.unwrap();
    let body = r.text().await.unwrap();
    assert_eq!(body.lines().count(), 24);
    assert!(serde_json::from_str::<Value>(body.lines().next().unwrap()).is_ok());
    // single hand + events
    let hid = h[0]["hand_id"].as_str().unwrap();
    assert_eq!(s.get(&format!("/v1/hands/{hid}"), Some("admin")).await.0, 200);
    let (st, ev) = s.get(&format!("/v1/hands/{hid}/events"), Some("admin")).await;
    assert_eq!(st, 200);
    assert!(ev.as_array().unwrap().iter().any(|e| e["type"] == "hand_end"));
    // leaderboard (min hands lowered to 6 in the test config)
    let (st, lb) = s.get("/v1/leaderboard", None).await;
    assert_eq!(st, 200);
    let rows = lb["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 6);
    assert!(rows.windows(2).all(|w| w[0]["score_mbb_per_hand"].as_f64() >= w[1]["score_mbb_per_hand"].as_f64()));
    assert!(rows[0]["ci95_mbb_per_hand"].as_f64().unwrap() > 0.0);
    let (_, lb2) = s.get("/v1/leaderboard?min_hands=100000", None).await;
    assert_eq!(lb2["rows"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn running_match_hands_are_private_and_matches_can_be_aborted() {
    let s = srv().await;
    let (_, tok) = s.register("spy").await;
    let (st, v) = s.post("/v1/matches", Some("admin"), json!({"participants": participants(), "hands_planned": 6, "duplicate": true})).await;
    assert_eq!(st, 201, "{v}");
    let id = v["match_id"].as_str().unwrap();
    assert_eq!(s.get(&format!("/v1/matches/{id}/hands"), Some(&tok)).await.0, 403);
    assert_eq!(s.post(&format!("/v1/matches/{id}/start"), Some(&tok), json!({})).await.0, 403);
    assert_eq!(s.post(&format!("/v1/matches/{id}/abort"), Some("admin"), json!({})).await.0, 200);
    let m = s.wait_match(id).await;
    assert_eq!(m["status"], "aborted");
    // bad duplicate size
    let (st, _) = s.post("/v1/matches", Some("admin"), json!({"participants": participants(), "hands_planned": 7, "duplicate": true})).await;
    assert_eq!(st, 400);
}


// ---- table_seq is gap-free per recipient ----

fn assert_gap_free(evs: &[(usize, Value)], clients: usize) {
    for c in 0..clients {
        let seqs: Vec<u64> = evs.iter().filter(|(i, m)| *i == c && m["table_seq"].is_u64()).map(|(_, m)| m["table_seq"].as_u64().unwrap()).collect();
        assert!(seqs.len() > 3, "client {c} saw {} sequenced messages", seqs.len());
        assert_eq!(seqs[0], 1, "client {c} starts at 1");
        for w in seqs.windows(2) {
            assert_eq!(w[1], w[0] + 1, "client {c}: table_seq gap {} -> {}", w[0], w[1]);
        }
    }
}

#[tokio::test]
async fn table_seq_is_gap_free_per_recipient() {
    let s = srv().await;
    let t = s.table(json!({"mode": "sandbox", "sandbox": {"button_seat": 3}})).await;
    let mut room = Room::new(&s, 6).await;
    room.sit_all(&s, &t, &[0, 1, 2, 3, 4, 5]).await;
    // everyone calls/checks to the river: private hole_cards, per-seat your_turn and public events interleave
    let evs = room.run_hand(&t, 6, |_, d| if d["legal"]["check"] == true { Some(("check", None)) } else { Some(("call", None)) }).await;
    assert_gap_free(&evs, 6);
    // each client got exactly one hole_cards (its own), so the global counter differs from theirs
    for c in 0..6 {
        assert_eq!(evs.iter().filter(|(i, m)| *i == c && m["type"] == "hole_cards").count(), 1);
    }
}

#[tokio::test]
async fn benchmark_mode_sends_only_your_turn_and_hand_end() {
    let s = srv().await;
    let t = s.table(json!({"mode": "benchmark", "sandbox": null, "decision_time_ms": 3000})).await;
    let mut room = Room::new(&s, 1).await;
    room.sit_all(&s, &t, &[0]).await;
    let (st, _) = s.post(&format!("/v1/tables/{t}/fill"), Some("admin"), json!({"bot": "house:calling_station"})).await;
    assert_eq!(st, 200);
    let mut all = vec![];
    for _ in 0..3 {
        all.extend(room.run_hand(&t, 1, |_, d| if d["legal"]["check"] == true { Some(("check", None)) } else { Some(("call", None)) }).await);
    }
    for (_, m) in &all {
        let ty = m["type"].as_str().unwrap();
        assert!(["your_turn", "hand_end", "seat_update", "welcome", "table_snapshot"].contains(&ty), "unexpected {ty} in benchmark mode");
    }
    assert!(all.iter().any(|(_, m)| m["type"] == "your_turn" && m["data"]["hole_cards"].as_array().unwrap().len() == 2));
    assert_eq!(all.iter().filter(|(_, m)| m["type"] == "hand_end").count(), 3);
    assert_gap_free(&all, 1);
    // nothing is lost: hands are stored (async writer) and readable
    let (st, hand) = s.get(&format!("/v1/hands/{t}-h1"), Some("admin")).await;
    assert_eq!(st, 200, "{hand}");
    assert_eq!(hand["results"].as_array().unwrap().len(), 6);
}

// ---- parallel lanes ----

#[tokio::test]
async fn lanes_play_every_hand_once_and_keep_decks_reproducible() {
    let s = srv().await;
    let mk = |lanes: usize| json!({"participants": participants(), "hands_planned": 72, "duplicate": true, "seed": "lanes",
                                   "block_deals": 2, "lanes": lanes, "table_config": {"mode": "benchmark"}});
    let mut ms = vec![];
    for lanes in [1, 3] {
        let (st, v) = s.post("/v1/matches", Some("admin"), mk(lanes)).await;
        assert_eq!(st, 201, "{v}");
        assert_eq!(v["table_ids"].as_array().unwrap().len(), lanes);
        let id = v["match_id"].as_str().unwrap().to_string();
        s.post(&format!("/v1/matches/{id}/start"), Some("admin"), json!({})).await;
        let m = s.wait_match(&id).await;
        assert_eq!(m["status"], "finished");
        assert_eq!(m["hands_played"], 72);
        let (_, h) = s.get(&format!("/v1/matches/{id}/hands?limit=500"), Some("admin")).await;
        ms.push((m, h.as_array().unwrap().clone()));
    }
    for (k, (_, h)) in ms.iter().enumerate() {
        let nos: Vec<u64> = h.iter().map(|x| x["hand_no"].as_u64().unwrap()).collect();
        assert_eq!(nos, (1..=72).collect::<Vec<u64>>(), "run {k}");
    }
    for i in 0..72 {
        assert_eq!(ms[0].1[i]["deck"], ms[1].1[i]["deck"]);
        assert_eq!(ms[0].1[i]["deal_no"], ms[1].1[i]["deal_no"]);
        assert_eq!(ms[0].1[i]["rotation"], ms[1].1[i]["rotation"]);
    }
    // the same duplicate scores regardless of lane count (house bots are seeded per deal)
    let s0: Vec<f64> = ms[0].0["standings"].as_array().unwrap().iter().map(|x| x["net_chips"].as_f64().unwrap()).collect();
    let s1: Vec<f64> = ms[1].0["standings"].as_array().unwrap().iter().map(|x| x["net_chips"].as_f64().unwrap()).collect();
    assert_eq!(s0, s1);
}

// ---- benchmark tracks ----

#[tokio::test]
async fn benchmark_track_needs_the_blueprint_and_lists_panels() {
    let s = srv().await;
    let (st, v) = s.get("/v1/benchmarks", None).await;
    assert_eq!(st, 200);
    assert_eq!(v.as_array().unwrap().len(), 2);
    assert_eq!(v[0]["track"], "ladder-v1");
    assert_eq!(v[0]["available"], false);
    assert_eq!(v[0]["default_hands"], 20000);
    let (st, e) = s.post("/v1/benchmarks/ladder-v1/runs", Some("admin"), json!({"player_id": "house:random", "hands": 60})).await;
    assert_eq!(st, 400, "{e}");
    assert_eq!(s.post("/v1/benchmarks/nope/runs", Some("admin"), json!({})).await.0, 404);
}

#[tokio::test]
async fn benchmark_runs_house_verified_remote_unverified_with_progress() {
    let Some(bp) = test_blueprint() else {
        eprintln!("skipped: no tiny blueprint (set SHARINGAN_TEST_BLUEPRINT)");
        return;
    };
    let s = srv_with(Some(bp)).await;
    // house bot: server-run, verified
    let (st, run) = s.post("/v1/benchmarks/ladder-v1/runs", Some("admin"), json!({"player_id": "house:tight_aggressive", "hands": 600})).await;
    assert_eq!(st, 201, "{run}");
    assert_eq!(run["verified"], true);
    assert_eq!(run["hands_planned"], 600);
    let rid = run["run_id"].as_str().unwrap().to_string();
    let mut done = Value::Null;
    for _ in 0..300 {
        let (_, r) = s.get(&format!("/v1/benchmarks/ladder-v1/runs/{rid}"), None).await;
        assert!(r["hands_total"] == 600);
        if r["status"] == "finished" {
            done = r;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(done["status"], "finished", "{done}");
    assert_eq!(done["hands_done"], 600);
    assert!(done["mbb_per_game"].is_number() && done["ci95_mbb"].as_f64().unwrap() > 0.0);
    assert_eq!(done["method"], "aivat");
    // a remote bot: unverified, gets benchmark_progress over its socket
    let mut room = Room::new(&s, 1).await;
    let (st, run2) = s
        .post("/v1/benchmarks/sharingan-1/runs", Some(&room.ids[0].1), json!({"hands": 120, "lanes": 2}))
        .await;
    assert_eq!(st, 201, "{run2}");
    assert_eq!(run2["verified"], false);
    let mut progress = 0;
    let mut final_status = String::new();
    while final_status.is_empty() {
        let (i, m) = room.recv().await;
        match m["type"].as_str().unwrap() {
            "your_turn" => {
                let d = m["data"].clone();
                let ty = if d["legal"]["check"] == true { "check" } else { "call" };
                room.act(i, m["table_id"].as_str().unwrap(), &d, ty, None);
            }
            "benchmark_progress" => {
                progress += 1;
                assert_eq!(m["data"]["hands_total"], 120);
                if m["data"]["status"] != "running" {
                    final_status = m["data"]["status"].as_str().unwrap().to_string();
                    assert_eq!(m["data"]["verified"], false);
                }
            }
            _ => {}
        }
    }
    assert_eq!(final_status, "finished");
    assert!(progress >= 1);
    // leaderboard: one column per track + verified flag, in the field names the site reads
    let (st, lb) = s.get("/v1/leaderboard?min_hands=100000", None).await;
    assert_eq!(st, 200);
    let rows = lb["rows"].as_array().unwrap();
    let house = rows.iter().find(|r| r["player_id"] == "house:tight_aggressive").expect("house row");
    assert_eq!(house["verified"], true);
    assert!(house["vs_ladder_v1"]["mbb"].is_number() && house["vs_ladder_v1"]["ci95_mbb"].is_number() && house["vs_ladder_v1"]["hands"] == 600);
    assert!(house["vs_sharingan_1"].is_null());
    let bot = rows.iter().find(|r| r["player_id"] == room.ids[0].0.as_str()).expect("bot row");
    assert_eq!(bot["verified"], false);
    assert!(bot["vs_sharingan_1"]["mbb"].is_number());
    let (_, only) = s.get("/v1/leaderboard?track=ladder-v1", None).await;
    assert_eq!(only["rows"].as_array().unwrap().len(), 1);
    let (_, t) = s.get("/v1/benchmarks/ladder-v1", None).await;
    assert_eq!(t["top_runs"].as_array().unwrap().len(), 1);
}
