//! Offline mock table: drives a [`Session`] through the real message path (`hand_start`,
//! `your_turn`, `player_action`, `hand_result`, ...) with five simple scripted opponents, no
//! network. Used by the smoke tests and by `openpoker offline` for latency measurement.
//!
//! Chips are Open Poker chips (blinds 10/20, stacks 2000 reset every hand); the mock keeps an
//! `nlhe` engine at x[`SCALE`] internally. The hero sits in seat 2 and the dealer rotates.

use crate::layout::{scale, unscale};
use crate::session::Session;
use crate::SCALE;
use cards::{card_str, Card, Rng};
use nlhe::{Action, Deal, GameConfig, State, Step};
use serde_json::{json, Value};

pub const HERO: usize = 2;

#[derive(Debug, Default)]
pub struct OfflineResult {
    pub hands: usize,
    /// Hero net result in Open Poker chips.
    pub hero_net: i64,
    /// Protocol-level problem (no/odd answer to a `your_turn`), if any.
    pub error: Option<String>,
}

fn names(stacks: &[i64; 6]) -> Vec<Value> {
    (0..6).map(|i| json!({"seat": i, "name": format!("bot{i}"), "stack": stacks[i]})).collect()
}

fn bot_action(st: &State, rng: &mut Rng) -> Action {
    let l = st.legal();
    let r = rng.f64();
    let want_raise = if l.can_fold { r >= 0.85 } else { r >= 0.65 };
    if want_raise && l.can_raise {
        if rng.f64() < 0.06 {
            return Action::RaiseTo(l.max_raise_to);
        }
        let frac = [0.5, 0.75, 1.0, 2.0][rng.below(4) as usize];
        let want = st.cur_bet() as f64 + frac * st.pot() as f64;
        let t = ((want / SCALE as f64).round() as i32 * SCALE).clamp(l.min_raise_to, l.max_raise_to);
        return Action::RaiseTo(t);
    }
    if l.can_fold {
        if r < 0.25 {
            Action::Fold
        } else {
            Action::Call
        }
    } else {
        Action::Check
    }
}

fn pa(op: usize, a: Action, st_before: &State, st_after: &State, nl: usize, street: &str) -> Value {
    let (name, amount) = match a {
        Action::Fold => ("fold", None),
        Action::Check => ("check", None),
        Action::Call => ("call", Some(unscale(st_before.legal().call_amount))),
        Action::RaiseTo(t) => ("raise", Some(unscale(t))),
    };
    json!({"type":"player_action","seat":op,"name":format!("bot{op}"),"action":name,"amount":amount,
           "street":street,"stack":unscale(st_after.stack(nl)),"pot":unscale(st_after.pot())})
}

fn street_name(s: nlhe::Street) -> &'static str {
    ["preflop", "flop", "turn", "river"][s.idx()]
}

/// Play `hands` hands. Stops early (with `error`) if the session misbehaves at the protocol level.
pub fn run(session: &mut Session, hands: usize, seed: u64) -> OfflineResult {
    let mut stacks = [2000i64; 6];
    let mut res = OfflineResult::default();
    let mut rng = Rng::new(seed ^ 0x6f66666c696e65);
    session.on_message(&json!({"type":"connected","agent_id":"a","name":"me"}));
    session.on_message(&json!({"type":"table_joined","table_id":"t1","seat":HERO,"players":names(&[2000; 6])}));
    for hn in 0..hands {
        match play_hand(session, hn, &mut stacks, &mut rng, &mut res) {
            Ok(()) => res.hands += 1,
            Err(e) => {
                res.error = Some(format!("hand {hn}: {e}"));
                break;
            }
        }
    }
    res
}

fn play_hand(session: &mut Session, hn: usize, stacks: &mut [i64; 6], rng: &mut Rng, res: &mut OfflineResult) -> Result<(), String> {
    // Stacks carry over; a seat that dropped below 10 big blinds rebuys to 2000 (announced).
    for (op, st) in stacks.iter_mut().enumerate() {
        if *st < 200 {
            *st = 2000;
            session.on_message(&json!({"type":"player_joined","seat":op,"name":format!("bot{op}"),"stack":2000}));
        }
    }
    let id = format!("h{hn}");
    let dealer = hn % 6;
    let op_of = |nl: usize| (dealer + 1 + nl) % 6;
    let nl_of = |op: usize| (op + 6 - dealer - 1) % 6;
    let hero_nl = nl_of(HERO);
    let deal = Deal::random(rng);
    let nl_stacks: Vec<i32> = (0..6).map(|nl| scale(stacks[op_of(nl)])).collect();
    let cfg = GameConfig::new(6).with_stacks(&nl_stacks);
    let mut st = State::new(&cfg);
    let cs = |cs: &[Card]| cs.iter().map(|&c| card_str(c)).collect::<Vec<_>>();
    session.on_message(&json!({"type":"hand_start","hand_id":id,"seat":HERO,"dealer_seat":dealer,"blinds":{"small_blind":10,"big_blind":20}}));
    session.on_message(&json!({"type":"hole_cards","cards":cs(&deal.holes[hero_nl])}));
    let mut shown = 0usize;
    let mut turn = 0usize;
    let players_json = |st: &State| -> Value {
        Value::Array(
            (0..6)
                .map(|op| {
                    let nl = nl_of(op);
                    let status = if st.is_folded(nl) { "folded" } else if st.is_all_in(nl) { "all_in" } else { "active" };
                    json!({"seat": op, "name": format!("bot{op}"), "stack": unscale(st.stack(nl)), "status": status})
                })
                .collect(),
        )
    };
    let mut step = Step::Continue;
    while let Some(nl) = st.to_act() {
        let op = op_of(nl);
        let before = st;
        let street = street_name(st.street());
        let a = if op == HERO {
            turn += 1;
            let l = st.legal();
            let mut va = vec![];
            if l.can_fold {
                va.push(json!({"action":"fold"}));
            }
            if l.can_check {
                va.push(json!({"action":"check"}));
            } else {
                va.push(json!({"action":"call","amount":unscale(l.call_amount)}));
            }
            if l.can_raise {
                va.push(json!({"action":"raise","min":unscale(l.min_raise_to),"max":unscale(l.max_raise_to)}));
            }
            let tok = format!("{id}-t{turn}");
            let board = cs(&deal.board[..st.street().board_len()]);
            let out = session.on_message(&json!({"type":"your_turn","hand_id":id,"turn_token":tok,"valid_actions":va,
                "pot":unscale(st.pot()),"community_cards":board,"players":players_json(&st),"min_raise":null,"max_raise":null}));
            let m = out.iter().find(|m| m["type"] == "action").ok_or("no action sent for your_turn")?.clone();
            let amt = m["amount"].as_i64();
            let a = match m["action"].as_str().unwrap_or("") {
                "fold" if l.can_fold => Action::Fold,
                "fold" | "check" => Action::Check,
                "call" => Action::Call,
                "raise" => Action::RaiseTo(scale(amt.ok_or("raise without amount")?).clamp(l.min_raise_to, l.max_raise_to)),
                "all_in" => Action::RaiseTo(l.max_raise_to),
                o => return Err(format!("unexpected action {o}")),
            };
            let a = if matches!(a, Action::RaiseTo(_)) && !l.can_raise { Action::Call } else { a };
            let a = if a == Action::Call && l.can_check { Action::Check } else { a };
            session.on_message(&json!({"type":"action_ack","client_action_id":m["client_action_id"],"status":"accepted"}));
            a
        } else {
            bot_action(&st, rng)
        };
        step = st.apply(a);
        session.on_message(&pa(op, a, &before, &st, nl, street));
        match step {
            Step::NewStreet(s) => {
                let to = s.board_len();
                session.on_message(&json!({"type":"community_cards","cards":cs(&deal.board[shown..to]),"street":street_name(s)}));
                shown = to;
            }
            Step::Terminal => break,
            Step::Continue => {}
        }
    }
    debug_assert!(step == Step::Terminal || st.is_terminal());
    // All-in run-out: the server deals the remaining board before the result.
    if st.showdown() {
        for to in [3usize, 4, 5] {
            if to > shown {
                session.on_message(&json!({"type":"community_cards","cards":cs(&deal.board[shown..to])}));
                shown = to;
            }
        }
    }
    let pay = st.payoffs(&deal);
    let mut fin = serde_json::Map::new();
    let mut payouts = vec![];
    let mut winners = vec![];
    for nl in 0..6 {
        let op = op_of(nl);
        let net = unscale(pay[nl]);
        stacks[op] += net;
        fin.insert(op.to_string(), json!(stacks[op]));
        let gross = unscale(pay[nl] + st.contributed(nl));
        if gross > 0 {
            payouts.push(json!({"seat": op, "amount": gross}));
            winners.push(json!({"seat": op, "name": format!("bot{op}"), "amount": net, "stack": stacks[op]}));
        }
    }
    res.hero_net += unscale(pay[hero_nl]);
    let shown_cards: serde_json::Map<String, Value> = if st.showdown() {
        (0..6).filter(|&nl| !st.is_folded(nl)).map(|nl| (op_of(nl).to_string(), json!(cs(&deal.holes[nl])))).collect()
    } else {
        Default::default()
    };
    session.on_message(&json!({"type":"hand_result","hand_id":id,"pot":unscale(st.pot()),"total_pot":unscale(st.pot()),
        "winners":winners,"payouts":payouts,"shown_cards":shown_cards,"final_stacks":fin}));
    Ok(())
}
