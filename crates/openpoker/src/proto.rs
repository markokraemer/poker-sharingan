//! Tolerant parsing of Open Poker server messages and builders for client messages.
//!
//! The docs (docs.openpoker.ai/llms-full.txt) fix the message names and the main fields, but
//! several payload shapes (`final_stacks`, `payouts`, `replayed_events`, `players[].status`)
//! are only loosely specified. Everything here accepts either shape we could plausibly see and
//! never panics on unexpected input; unknown pieces simply come back as `None`.

use cards::{parse_card, Card};
use serde_json::{json, Value};

pub fn as_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.round() as i64)),
        Value::String(s) => s.trim().parse::<f64>().ok().map(|f| f.round() as i64),
        _ => None,
    }
}

pub fn get_i64(v: &Value, k: &str) -> Option<i64> {
    v.get(k).and_then(as_i64)
}

pub fn get_u64(v: &Value, k: &str) -> Option<u64> {
    v.get(k).and_then(as_i64).filter(|x| *x >= 0).map(|x| x as u64)
}

pub fn get_str<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

pub fn msg_type(v: &Value) -> &str {
    get_str(v, "type").unwrap_or("")
}

/// `"Ah"`, `"10h"`, `"Th"` -> card.
pub fn parse_op_card(s: &str) -> Option<Card> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("10") {
        return parse_card(&format!("T{rest}"));
    }
    parse_card(s)
}

pub fn parse_cards_value(v: Option<&Value>) -> Vec<Card> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|c| c.as_str().and_then(parse_op_card))
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Clone, Debug, PartialEq)]
pub struct ValidAction {
    pub action: String,
    pub amount: Option<i64>,
    pub min: Option<i64>,
    pub max: Option<i64>,
}

pub fn parse_valid_actions(v: Option<&Value>) -> Vec<ValidAction> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    // Either {"action": "call", "amount": 20} or a bare string "fold".
                    if let Some(s) = x.as_str() {
                        return Some(ValidAction {
                            action: s.to_string(),
                            amount: None,
                            min: None,
                            max: None,
                        });
                    }
                    Some(ValidAction {
                        action: get_str(x, "action")?.to_string(),
                        amount: get_i64(x, "amount"),
                        min: get_i64(x, "min"),
                        max: get_i64(x, "max"),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What the server allows right now, in Open Poker chips.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ValidSet {
    pub fold: bool,
    pub check: bool,
    /// Chips to call (as advertised; `Some(0)` if advertised without amount).
    pub call: Option<i64>,
    /// (min, max) raise-to.
    pub raise: Option<(i64, i64)>,
    /// All-in (distinct from raise): its amount if advertised.
    pub all_in: Option<Option<i64>>,
}

impl ValidSet {
    pub fn from_actions(va: &[ValidAction], min_raise: Option<i64>, max_raise: Option<i64>) -> Self {
        let mut s = ValidSet::default();
        for a in va {
            match a.action.as_str() {
                "fold" => s.fold = true,
                "check" => s.check = true,
                "call" => s.call = Some(a.amount.unwrap_or(0)),
                "raise" | "bet" => {
                    let lo = a.min.or(min_raise).unwrap_or(0);
                    let hi = a.max.or(max_raise).unwrap_or(lo);
                    s.raise = Some((lo, hi.max(lo)));
                }
                "all_in" | "allin" => s.all_in = Some(a.amount),
                _ => {}
            }
        }
        s
    }
    pub fn is_empty(&self) -> bool {
        !self.fold && !self.check && self.call.is_none() && self.raise.is_none() && self.all_in.is_none()
    }
    pub fn describe(&self) -> Vec<String> {
        let mut v = Vec::new();
        if self.fold {
            v.push("fold".into());
        }
        if self.check {
            v.push("check".into());
        }
        if let Some(c) = self.call {
            v.push(format!("call:{c}"));
        }
        if let Some((a, b)) = self.raise {
            v.push(format!("raise:{a}-{b}"));
        }
        if let Some(a) = self.all_in {
            v.push(format!("all_in:{}", a.map_or("?".into(), |x| x.to_string())));
        }
        v
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerInfo {
    pub seat: usize,
    pub name: Option<String>,
    pub stack: Option<i64>,
    /// Chips committed on the current street (`table_state.seats[].bet`).
    pub bet: Option<i64>,
    pub in_hand: Option<bool>,
    pub status: Option<String>,
}

impl PlayerInfo {
    /// Was this seat dealt into the current hand (as far as the message tells)?
    /// `None` if the message does not say.
    pub fn dealt_in(&self) -> Option<bool> {
        if let Some(s) = &self.status {
            let s = s.to_ascii_lowercase();
            if matches!(
                s.as_str(),
                "sitting_out" | "sit_out" | "sitout" | "empty" | "out" | "busted" | "left" | "away" | "waiting" | "disconnected"
            ) {
                return Some(false);
            }
            if matches!(s.as_str(), "folded" | "active" | "all_in" | "allin" | "in_hand" | "playing" | "acting" | "to_act") {
                return Some(true);
            }
        }
        // `in_hand == true` is conclusive; `false` is ambiguous (folded vs sitting out).
        match self.in_hand {
            Some(true) => Some(true),
            _ => None,
        }
    }
    pub fn folded_hint(&self) -> bool {
        self.status.as_deref().is_some_and(|s| s.eq_ignore_ascii_case("folded"))
    }
}

pub fn parse_players(v: Option<&Value>) -> Vec<PlayerInfo> {
    let mut out = Vec::new();
    let Some(a) = v.and_then(Value::as_array) else {
        return out;
    };
    for p in a {
        // Entries may be null for empty seats in `table_state.seats`.
        let Some(seat) = p.get("seat").and_then(as_i64) else {
            continue;
        };
        if !(0..64).contains(&seat) {
            continue;
        }
        out.push(PlayerInfo {
            seat: seat as usize,
            name: get_str(p, "name").map(str::to_string),
            stack: get_i64(p, "stack"),
            bet: get_i64(p, "bet"),
            in_hand: p.get("in_hand").and_then(Value::as_bool).or_else(|| {
                p.get("folded").and_then(Value::as_bool).map(|f| !f)
            }),
            status: get_str(p, "status").map(str::to_string),
        });
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObsKind {
    Fold,
    Check,
    Call,
    Raise,
    AllIn,
    /// Blind posts and anything we do not model.
    Ignored,
}

/// A `player_action` broadcast (also the form of entries in `hand_result.actions`).
#[derive(Clone, Debug, PartialEq)]
pub struct ActionObs {
    pub seat: usize,
    pub name: Option<String>,
    pub kind: ObsKind,
    pub raw_action: String,
    pub amount: Option<i64>,
    pub street: Option<String>,
    /// Actor's stack after the action.
    pub stack: Option<i64>,
    /// Actor's stack before the action (after any blind already posted).
    pub stack_before: Option<i64>,
    /// Chips this action moved into the pot.
    pub delta: Option<i64>,
    /// Pot after the action.
    pub pot: Option<i64>,
    /// `"to_total"` (raise-to on the street) or `"incremental"`.
    pub amount_mode: Option<String>,
}

pub fn parse_obs_kind(s: &str) -> ObsKind {
    match s.to_ascii_lowercase().as_str() {
        "fold" => ObsKind::Fold,
        "check" => ObsKind::Check,
        "call" => ObsKind::Call,
        "raise" | "bet" => ObsKind::Raise,
        "all_in" | "allin" | "all-in" => ObsKind::AllIn,
        _ => ObsKind::Ignored,
    }
}

pub fn parse_player_action(v: &Value) -> Option<ActionObs> {
    let seat = v.get("seat").and_then(as_i64)?;
    let raw = get_str(v, "action")?.to_string();
    Some(ActionObs {
        seat: usize::try_from(seat).ok()?,
        name: get_str(v, "name").map(str::to_string),
        kind: parse_obs_kind(&raw),
        raw_action: raw,
        amount: get_i64(v, "amount"),
        street: get_str(v, "street").map(str::to_string),
        stack: get_i64(v, "stack_after").or_else(|| get_i64(v, "stack")),
        stack_before: get_i64(v, "stack_before"),
        delta: get_i64(v, "contribution_delta"),
        pot: get_i64(v, "pot_after").or_else(|| get_i64(v, "pot")),
        amount_mode: get_str(v, "amount_mode").map(str::to_string),
    })
}

/// `{"1": 1990, "3": 2010}` or `[{"seat":1,"stack":1990}, ...]` -> (seat, stack) pairs.
pub fn parse_seat_numbers(v: Option<&Value>, value_keys: &[&str]) -> Vec<(usize, i64)> {
    let mut out = Vec::new();
    match v {
        Some(Value::Object(m)) => {
            for (k, x) in m {
                if let (Ok(seat), Some(n)) = (k.parse::<usize>(), as_i64(x)) {
                    out.push((seat, n));
                }
            }
        }
        Some(Value::Array(a)) => {
            for x in a {
                let Some(seat) = x.get("seat").and_then(as_i64) else { continue };
                let n = value_keys.iter().find_map(|k| x.get(*k).and_then(as_i64));
                if let (Ok(seat), Some(n)) = (usize::try_from(seat), n) {
                    out.push((seat, n));
                }
            }
        }
        _ => {}
    }
    out.sort_unstable();
    out
}

// ---- client messages ----

pub fn join_lobby(buy_in: i64) -> Value {
    json!({"type": "join_lobby", "buy_in": buy_in})
}
pub fn set_auto_rebuy(enabled: bool) -> Value {
    json!({"type": "set_auto_rebuy", "enabled": enabled})
}
pub fn rebuy() -> Value {
    json!({"type": "rebuy", "amount": 1500})
}
pub fn leave_table() -> Value {
    json!({"type": "leave_table"})
}
pub fn resync_request(table_id: &str, last_table_seq: u64) -> Value {
    json!({"type": "resync_request", "table_id": table_id, "last_table_seq": last_table_seq})
}
pub fn action_msg(hand_id: &str, action: &str, amount: Option<i64>, client_action_id: &str, turn_token: &str) -> Value {
    let mut m = json!({
        "type": "action",
        "hand_id": hand_id,
        "action": action,
        "client_action_id": client_action_id,
        "turn_token": turn_token,
    });
    if let Some(a) = amount {
        m["amount"] = json!(a);
    }
    m
}

/// Codes that mean "your action was not accepted".
pub fn is_rejection_code(code: &str) -> bool {
    matches!(
        code,
        "action_rejected"
            | "not_your_turn"
            | "stale_hand_action"
            | "stale_turn_token"
            | "invalid_action"
            | "legacy_action_protocol"
            | "missing_action_id"
            | "action_id_conflict"
            | "no_hand_in_progress"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_parse_with_ten() {
        assert_eq!(parse_op_card("10h"), parse_card("Th"));
        assert_eq!(parse_op_card("Ah"), parse_card("Ah"));
        assert_eq!(parse_op_card("zz"), None);
        let v = json!(["Ah", "Kd", "bad"]);
        assert_eq!(parse_cards_value(Some(&v)).len(), 2);
    }

    #[test]
    fn valid_actions_shapes() {
        let v = json!([{"action":"fold"},{"action":"call","amount":40.0},{"action":"raise","min":80,"max":1990},{"action":"all_in","amount":1990}]);
        let s = ValidSet::from_actions(&parse_valid_actions(Some(&v)), None, None);
        assert!(s.fold && !s.check);
        assert_eq!(s.call, Some(40));
        assert_eq!(s.raise, Some((80, 1990)));
        assert_eq!(s.all_in, Some(Some(1990)));
        let v = json!(["check", "fold"]);
        let s = ValidSet::from_actions(&parse_valid_actions(Some(&v)), None, None);
        assert!(s.check && s.fold);
        // min/max fall back to top-level fields
        let v = json!([{"action":"raise"}]);
        let s = ValidSet::from_actions(&parse_valid_actions(Some(&v)), Some(40), Some(500));
        assert_eq!(s.raise, Some((40, 500)));
    }

    #[test]
    fn player_status_semantics() {
        let v = json!([
            {"seat":0,"stack":1000,"status":"sitting_out"},
            {"seat":1,"stack":1000,"status":"folded"},
            {"seat":2,"stack":1000,"in_hand":true},
            {"seat":3,"stack":1000,"in_hand":false},
            null,
            {"seat":4,"stack":0,"status":"all_in"}
        ]);
        let p = parse_players(Some(&v));
        assert_eq!(p.len(), 5);
        assert_eq!(p[0].dealt_in(), Some(false));
        assert_eq!(p[1].dealt_in(), Some(true));
        assert!(p[1].folded_hint());
        assert_eq!(p[2].dealt_in(), Some(true));
        assert_eq!(p[3].dealt_in(), None);
        assert_eq!(p[4].dealt_in(), Some(true));
    }

    #[test]
    fn player_action_and_seat_numbers() {
        let v = json!({"type":"player_action","seat":3,"name":"x","action":"raise","amount":60,"street":"preflop","stack":1940,"pot":90});
        let a = parse_player_action(&v).unwrap();
        assert_eq!((a.seat, a.kind, a.amount, a.stack, a.pot), (3, ObsKind::Raise, Some(60), Some(1940), Some(90)));
        let a = parse_player_action(&json!({"seat":1,"action":"small_blind","amount":10})).unwrap();
        assert_eq!(a.kind, ObsKind::Ignored);
        let m = parse_seat_numbers(Some(&json!({"1": 1990, "3": "2010"})), &["stack"]);
        assert_eq!(m, vec![(1, 1990), (3, 2010)]);
        let m = parse_seat_numbers(Some(&json!([{"seat":2,"stack":5},{"seat":0,"final_stack":7}])), &["stack", "final_stack"]);
        assert_eq!(m, vec![(0, 7), (2, 5)]);
    }
}
