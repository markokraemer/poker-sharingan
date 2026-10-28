//! Shared test kit: a tiny-blueprint `Core`, an independent random-hand generator (its own nlhe
//! oracle) and an in-process HTTP client.

#![allow(dead_code)]

use axum::body::Body;
use axum::http::{HeaderMap, Request};
use axum::Router;
use cards::{card_str, Rng};
use http_body_util::BodyExt;
use nlhe::{Action, GameConfig, State, Step, MAX_PLAYERS};
use serde_json::{json, Value};
use sharingan_serve::config::Config;
use sharingan_serve::decide::Core;
use sharingan_serve::server::{router, AppState};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use tower::ServiceExt;

pub const KEY: &str = "k-alice";
pub const KEY2: &str = "k-bob";

pub fn blueprint_path() -> Option<PathBuf> {
    let p = std::env::var("SHARINGAN_TEST_BLUEPRINT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("dev-serve/tinybp/tiny.plcp"));
    p.exists().then_some(p)
}

pub fn base_cfg() -> Option<Config> {
    let blueprint = blueprint_path()?;
    let buckets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/buckets-200");
    Some(Config {
        blueprint,
        buckets: buckets.display().to_string(),
        sizing: "tiny".into(),
        sets: "flop=0,iters=1500".into(),
        search_threads: 8,
        search_concurrency: 2,
        budget_ms: 3000,
        queue_ms: 2000,
        api_keys: vec![("alice".into(), KEY.into()), ("bob".into(), KEY2.into())],
        rate_rps: 100_000.0,
        rate_burst: 100_000.0,
        ..Config::default()
    })
}

fn load(cfg: Option<Config>) -> Option<Arc<Core>> {
    let cfg = cfg?;
    sharingan_serve::log::set_enabled(false);
    Some(Arc::new(Core::new(&cfg).expect("load tiny blueprint")))
}

/// Shared default core (search on turn/river with 1500 iterations, 3 s budget).
pub fn core_main() -> Option<Arc<Core>> {
    static C: OnceLock<Option<Arc<Core>>> = OnceLock::new();
    C.get_or_init(|| load(base_cfg())).clone()
}

/// One search slot, every search sleeps 1.5 s first, 300 ms budget, no queueing: forces timeouts.
pub fn core_slow() -> Option<Arc<Core>> {
    static C: OnceLock<Option<Arc<Core>>> = OnceLock::new();
    C.get_or_init(|| {
        load(base_cfg().map(|mut c| {
            c.search_concurrency = 1;
            c.budget_ms = 300;
            c.queue_ms = 0;
            c.debug_search_delay_ms = 1500;
            c
        }))
    })
    .clone()
}

/// Strict rate limit: 1 req/s, burst 5.
pub fn core_limited() -> Option<Arc<Core>> {
    static C: OnceLock<Option<Arc<Core>>> = OnceLock::new();
    C.get_or_init(|| {
        load(base_cfg().map(|mut c| {
            c.rate_rps = 1.0;
            c.rate_burst = 5.0;
            c.max_batch = 8;
            c
        }))
    })
    .clone()
}

pub fn app(core: &Arc<Core>) -> Router {
    router(AppState::new(core.clone()))
}

pub async fn call(app: &Router, method: &str, path: &str, key: Option<&str>, body: &str) -> (u16, HeaderMap, Value) {
    let mut rq = Request::builder().method(method).uri(path).header("content-type", "application/json");
    if let Some(k) = key {
        rq = rq.header("authorization", format!("Bearer {k}"));
    }
    let resp = app.clone().oneshot(rq.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let (parts, body) = resp.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    let v = serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (parts.status.as_u16(), parts.headers, v)
}

// ------------------------------------------------------------------------------ generator

/// What an independent nlhe oracle says about the hero's decision.
#[derive(Clone, Debug)]
pub struct Expect {
    pub street: usize,
    pub can_fold: bool,
    pub can_check: bool,
    pub call_to: Option<i64>,
    pub raise: Option<(i64, i64)>,
    pub hero_bet: i64,
    pub hero_prior: i64,
    pub bb: i64,
    pub n: usize,
}

#[derive(Clone, Debug)]
pub struct Spot {
    pub req: Value,
    pub expect: Expect,
    /// Index into `req["history"]` of each raise/bet event with its legal (min, max) at that point.
    pub raise_events: Vec<(usize, i64, i64)>,
    /// History indices of calls made while facing a bet.
    pub call_events: Vec<usize>,
}

#[derive(Clone, Copy)]
pub struct GenOpts {
    pub deep: bool,
    pub bbs: &'static [i64],
}

pub const BBS: &[i64] = &[100, 100, 100, 20, 2, 50, 200, 25, 10, 1000, 30, 1];

pub fn gen_hand(rng: &mut Rng, o: GenOpts) -> Vec<Spot> {
    let n = 2 + rng.below(5) as usize;
    let max_seat = 7u32;
    let mut pool: Vec<usize> = (0..=max_seat as usize).collect();
    for i in (1..pool.len()).rev() {
        pool.swap(i, rng.below(i as u32 + 1) as usize);
    }
    let mut seats: Vec<usize> = pool[..n].to_vec();
    let sitting_out: Vec<usize> = pool[n..n + rng.below(3) as usize].to_vec();
    seats.sort_unstable();
    let button = rng.below(max_seat + 1) as usize;
    let bb = o.bbs[rng.below(o.bbs.len() as u32) as usize];
    let sb = (bb / 2).max(1);
    let mut stacks = vec![0i64; n];
    for s in stacks.iter_mut() {
        *s = match rng.below(100) {
            0..=69 => bb * (if o.deep { 100 + rng.below(900) as i64 } else { 20 + rng.below(230) as i64 }),
            70..=84 => 1 + rng.below((bb * 6) as u32) as i64,
            _ => bb * 100,
        };
        *s = (*s).max(1);
    }
    let hero_i = rng.below(n as u32) as usize;
    let hero = seats[hero_i];
    // independent seat order: SB = first dealt seat after the button, ..., heads-up the button is the SB
    let start = seats.iter().position(|&s| s > button).unwrap_or(0);
    let mut order: Vec<usize> = (0..n).map(|i| seats[(start + i) % n]).collect();
    if n == 2 && seats.contains(&button) {
        order = vec![button, *seats.iter().find(|&&s| s != button).unwrap()];
    }
    let stack_of = |seat: usize| stacks[seats.iter().position(|&s| s == seat).unwrap()];
    let mut arr = [0i32; MAX_PLAYERS];
    for (ri, &seat) in order.iter().enumerate() {
        arr[ri] = stack_of(seat) as i32;
    }
    let cfg = GameConfig { players: n, stack: 0, small_blind: sb as i32, big_blind: bb as i32, stacks: Some(arr) };
    let mut st = State::new(&cfg);
    let deck = rng.deck();
    let hero_ri = order.iter().position(|&s| s == hero).unwrap();
    let hole = [deck[2 * hero_ri], deck[2 * hero_ri + 1]];
    let board_all: Vec<_> = deck[20..25].to_vec();
    let mut players: Vec<Value> = seats.iter().map(|&s| json!({"seat": s, "stack_start": stack_of(s)})).collect();
    players.extend(sitting_out.iter().map(|&s| json!({"seat": s, "stack_start": bb * 100, "in_hand": false})));
    let mut hist: Vec<Value> = vec![];
    if rng.below(2) == 0 {
        hist.push(json!({"street": "preflop", "seat": order[0], "type": "post_sb", "to": sb.min(stack_of(order[0]))}));
        hist.push(json!({"street": "preflop", "seat": order[1], "type": "post_bb", "to": bb.min(stack_of(order[1]))}));
    }
    let mut spots = vec![];
    let mut raise_events = vec![];
    let mut call_events = vec![];
    let streets = ["preflop", "flop", "turn", "river"];
    while !st.is_terminal() {
        let ri = st.to_act().unwrap();
        let seat = order[ri];
        let l = st.legal();
        let street = st.street();
        let board: Vec<String> = board_all[..street.board_len()].iter().map(|&c| card_str(c)).collect();
        if seat == hero {
            let bet = st.street_bet(ri) as i64;
            spots.push(Spot {
                req: json!({
                    "hand_id": format!("h-{}", rng.below(1_000_000)),
                    "small_blind": sb, "big_blind": bb, "button_seat": button, "hero_seat": hero,
                    "hole_cards": [card_str(hole[0]), card_str(hole[1])],
                    "board": board, "players": players, "history": hist,
                }),
                expect: Expect {
                    street: street.idx(),
                    can_fold: l.can_fold,
                    can_check: l.can_check,
                    call_to: (!l.can_check).then(|| bet + l.call_amount as i64),
                    raise: l.can_raise.then(|| (l.min_raise_to as i64, l.max_raise_to as i64)),
                    hero_bet: bet,
                    hero_prior: st.contributed(ri) as i64 - bet,
                    bb,
                    n,
                },
                raise_events: raise_events.clone(),
                call_events: call_events.clone(),
            });
        }
        // random continuation (the hero's own action is random too)
        let r = rng.below(100);
        let a = if r < 20 && l.can_fold {
            Action::Fold
        } else if r < 72 || !l.can_raise {
            if l.can_check {
                Action::Check
            } else {
                Action::Call
            }
        } else {
            let t = match rng.below(10) {
                0..=4 => l.min_raise_to,
                5..=6 => l.max_raise_to,
                _ => l.min_raise_to + rng.below((l.max_raise_to - l.min_raise_to + 1) as u32) as i32,
            };
            Action::RaiseTo(t)
        };
        let bet_before = st.cur_bet();
        let mut ev = json!({"seat": seat});
        if rng.below(2) == 0 {
            ev["street"] = json!(streets[street.idx()]);
        }
        match a {
            Action::Fold => ev["type"] = json!("fold"),
            Action::Check => ev["type"] = json!("check"),
            Action::Call => {
                ev["type"] = json!("call");
                call_events.push(hist.len());
                if rng.below(2) == 0 {
                    ev["to"] = json!(st.street_bet(ri) + l.call_amount);
                }
            }
            Action::RaiseTo(t) => {
                raise_events.push((hist.len(), l.min_raise_to as i64, l.max_raise_to as i64));
                if t == l.max_raise_to && rng.below(2) == 0 {
                    ev["type"] = json!("all_in");
                    if rng.below(2) == 0 {
                        ev["to"] = json!(t);
                    }
                } else {
                    ev["type"] = json!(if bet_before == 0 && rng.below(2) == 0 { "bet" } else { "raise" });
                    ev["to"] = json!(t);
                }
            }
        }
        hist.push(ev);
        match st.apply(a) {
            Step::Terminal => break,
            _ => {}
        }
    }
    spots
}

pub fn gen_spots(seed: u64, want: usize, o: GenOpts, pred: &dyn Fn(&Spot) -> bool) -> Vec<Spot> {
    let mut rng = Rng::new(seed);
    let mut out = vec![];
    let mut guard = 0;
    while out.len() < want && guard < 200_000 {
        guard += 1;
        let hand = gen_hand(&mut rng, o);
        // at most 2 spots per hand, to spread over many different hands
        let mut ok: Vec<Spot> = hand.into_iter().filter(|s| pred(s)).collect();
        for _ in 0..2 {
            if !ok.is_empty() {
                out.push(ok.swap_remove(rng.below(ok.len() as u32) as usize));
            }
        }
    }
    out
}

pub const STD: GenOpts = GenOpts { deep: false, bbs: BBS };

/// Check a 200 response against the oracle. Panics with the request on failure.
pub fn check_response(spot: &Spot, resp: &Value) {
    let ctx = || format!("request {} -> response {}", spot.req, resp);
    let e = &spot.expect;
    let a = &resp["action"];
    let kind = a["type"].as_str().unwrap_or_else(|| panic!("no action type: {}", ctx()));
    let legal = &resp["legal"];
    assert_eq!(legal["fold"].as_bool().unwrap(), e.can_fold, "{}", ctx());
    assert_eq!(legal["check"].as_bool().unwrap(), e.can_check, "{}", ctx());
    assert_eq!(legal["call"]["to"].as_i64(), e.call_to, "{}", ctx());
    assert_eq!(legal["raise"]["min_to"].as_i64(), e.raise.map(|r| r.0), "{}", ctx());
    assert_eq!(legal["raise"]["max_to"].as_i64(), e.raise.map(|r| r.1), "{}", ctx());
    match kind {
        "fold" => assert!(e.can_fold, "{}", ctx()),
        "check" => assert!(e.can_check, "{}", ctx()),
        "call" => {
            assert!(!e.can_check, "{}", ctx());
            assert_eq!(a["to"].as_i64(), e.call_to, "{}", ctx());
            assert_eq!(a["added"].as_i64().unwrap(), e.call_to.unwrap() - e.hero_bet, "{}", ctx());
        }
        "raise" => {
            let (lo, hi) = e.raise.unwrap_or_else(|| panic!("raise not legal: {}", ctx()));
            let to = a["to"].as_i64().unwrap();
            assert!(to >= lo && to <= hi, "raise to {to} outside [{lo},{hi}]: {}", ctx());
            assert_eq!(a["added"].as_i64().unwrap(), to - e.hero_bet, "{}", ctx());
            assert_eq!(a["all_in"].as_bool().unwrap(), to == hi, "{}", ctx());
        }
        k => panic!("unknown action {k}: {}", ctx()),
    }
    if let Some(p) = resp["probs"].as_array() {
        let sum: f64 = p.iter().map(|x| x["p"].as_f64().unwrap()).sum();
        if resp["fallback_reason"].is_null() {
            assert!((sum - 1.0).abs() < 2e-3, "probs sum {sum}: {}", ctx());
        }
        for x in p {
            match x["type"].as_str().unwrap() {
                "fold" => assert!(e.can_fold, "{}", ctx()),
                "check" => assert!(e.can_check, "{}", ctx()),
                "call" => assert_eq!(x["to"].as_i64(), e.call_to, "{}", ctx()),
                "raise" => {
                    let (lo, hi) = e.raise.unwrap_or_else(|| panic!("raise prob but not legal: {}", ctx()));
                    let to = x["to"].as_i64().unwrap();
                    assert!(to >= lo && to <= hi, "prob raise to {to} outside [{lo},{hi}]: {}", ctx());
                }
                k => panic!("{k}"),
            }
        }
    }
    assert!(resp["timing"]["total_ms"].as_f64().unwrap() >= 0.0);
}

/// River spots (from `seed`) whose hero history is on-policy, i.e. that the main core really
/// searches (random hero actions are mostly off-policy and now skip the search). Returns `want`.
pub async fn searchable_river_spots(seed: u64, want: usize) -> Vec<Spot> {
    let core = core_main().expect("tiny blueprint");
    let a = app(&core);
    let mut out = vec![];
    for sp in gen_spots(seed, 400, STD, &|s| s.expect.street == 3 && s.expect.n >= 2) {
        let (_, _, v) = call(&a, "POST", "/v1/act", Some(KEY), &sp.req.to_string()).await;
        if v["source"] == "search" {
            out.push(sp);
            if out.len() == want {
                break;
            }
        }
    }
    assert_eq!(out.len(), want, "not enough searchable river spots");
    out
}
