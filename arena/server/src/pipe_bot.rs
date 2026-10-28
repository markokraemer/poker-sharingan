//! `sharingan-arena pipe-house <name>`: a house bot (random, calling_station, tight_aggressive, ...,
//! sharingan-1 with ARENA_BLUEPRINT) that speaks the PIPE protocol (TABLE_API.md section 14) on
//! stdin/stdout. It rebuilds the engine state from each `your_turn` (players + history), so it is a
//! faithful reference implementation of a stateless pipe bot and the "Rust house bot" of the
//! throughput benchmark: `sharingan-arena run --bot "sharingan-arena pipe-house tight_aggressive" ...`.

use crate::config::ServerConfig;
use crate::house::HouseRegistry;
use crate::util::seed64;
use bench::mix;
use bench::policy::{Event, View};
use nlhe::{Action, GameConfig, State, Step};
use serde_json::{json, Value};
use std::io::{BufRead, Write};

fn parse_action(kind: &str, to: i64) -> Action {
    match kind {
        "fold" => Action::Fold,
        "check" => Action::Check,
        "call" => Action::Call,
        _ => Action::RaiseTo(to as i32),
    }
}

/// Decide a `your_turn` `data` object; returns the `act` data.
pub fn decide(house: &HouseRegistry, key: &str, data: &Value) -> Option<Value> {
    let factory = house.get(key)?;
    let hand_id = data["hand_id"].as_str().unwrap_or("");
    let players = data["players"].as_array()?;
    let n = players.len();
    let button = data["button_seat"].as_u64()? as usize;
    let me = data["seat"].as_u64()? as usize;
    let occupied: Vec<usize> = players.iter().filter_map(|p| p["seat"].as_u64().map(|s| s as usize)).collect();
    let order: Vec<usize> = if n == 2 {
        vec![button, *occupied.iter().find(|&&s| s != button)?]
    } else {
        (1..=6).map(|k| (button + k) % 6).filter(|s| occupied.contains(s)).collect()
    };
    let stack_of = |seat: usize| players.iter().find(|p| p["seat"].as_u64() == Some(seat as u64)).and_then(|p| p["stack_start"].as_i64()).unwrap_or(0);
    let stacks: Vec<i32> = order.iter().map(|&s| stack_of(s) as i32).collect();
    let ecfg = GameConfig {
        players: n,
        stack: stacks.iter().copied().max().unwrap_or(10_000),
        small_blind: data["small_blind"].as_i64()? as i32,
        big_blind: data["big_blind"].as_i64()? as i32,
        stacks: None,
    }
    .with_stacks(&stacks);
    let idx = order.iter().position(|&s| s == me)?;
    let board: Vec<u8> = data["board"].as_array()?.iter().filter_map(|c| c.as_str().and_then(cards::parse_card)).collect();
    let hc = data["hole_cards"].as_array()?;
    let hole = [cards::parse_card(hc.first()?.as_str()?)?, cards::parse_card(hc.get(1)?.as_str()?)?];
    let hseed = seed64(hand_id);
    let mut p = factory.new_player(idx, &ecfg, mix(hseed, idx as u64));
    let mut st = State::new(&ecfg);
    let mut hist: Vec<Event> = Vec::new();
    for a in data["history"].as_array()? {
        if a["forced"].as_bool() == Some(true) {
            continue;
        }
        let seat = a["seat"].as_u64()? as usize;
        let ai = order.iter().position(|&s| s == seat)?;
        if st.to_act() != Some(ai) {
            return None;
        }
        let act = parse_action(a["type"].as_str()?, a["to"].as_i64().unwrap_or(0));
        let pre = st;
        let street = st.street();
        let step = st.apply(act);
        hist.push(Event { seat: ai as u8, street, action: act });
        p.observe(&pre, ai, act);
        if let Step::NewStreet(s) = step {
            p.observe_board(&board[..s.board_len().min(board.len())]);
        }
    }
    if st.to_act() != Some(idx) {
        return None;
    }
    let u = cards::Rng::new(mix(hseed, hist.len() as u64)).f64();
    let action = {
        let view = View { cfg: &ecfg, state: &st, board: &board, history: &hist, seat: idx };
        p.act(&view, hole, u)
    };
    let (kind, to) = match action {
        Action::Fold => ("fold", None),
        Action::Check => ("check", None),
        Action::Call => ("call", None),
        Action::RaiseTo(t) => (if st.cur_bet() == 0 { "bet" } else { "raise" }, Some(t)),
    };
    let mut out = json!({"hand_id": hand_id, "decision_id": data["decision_id"], "type": kind});
    if let Some(t) = to {
        out["to"] = json!(t);
    }
    Some(out)
}

pub fn cli_pipe_house(args: Vec<String>) -> i32 {
    let Some(name) = args.first() else {
        eprintln!("usage: sharingan-arena pipe-house <random|calling_station|tight_aggressive|nit|lag|maniac|sharingan-1>");
        return 2;
    };
    let house = match HouseRegistry::build(&ServerConfig::from_env()) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("pipe-house: {e}");
            return 1;
        }
    };
    let Some(key) = house.key_of(name) else {
        eprintln!("pipe-house: unknown house bot {name}");
        return 2;
    };
    let stdin = std::io::stdin();
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    let hello = json!({"v": 1, "type": "hello", "data": {"client": format!("sharingan-pipe-house/{}", env!("CARGO_PKG_VERSION")), "protocol": 1, "bot": key}});
    let _ = writeln!(out, "{hello}");
    let _ = out.flush();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        match v["type"].as_str() {
            Some("your_turn") => {
                let act = decide(&house, &key, &v["data"]).unwrap_or_else(|| {
                    json!({"hand_id": v["data"]["hand_id"], "decision_id": v["data"]["decision_id"], "type": "fold"})
                });
                let env = json!({"v": 1, "type": "act", "table_id": v["table_id"], "t_ms": crate::util::now_ms(), "data": act});
                if writeln!(out, "{env}").is_err() || out.flush().is_err() {
                    break;
                }
            }
            Some("bye") => break,
            _ => {}
        }
    }
    0
}
