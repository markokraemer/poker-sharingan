//! Heads-up Slumbot adapter on the nlhe engine (200 bb stacks = 20,000 chips, blinds 50/100).
//!
//! API: POST `{host}/slumbot/api/new_hand` and `/act` (`{"token","incr"}`). Action strings:
//! `k` check, `c` call, `f` fold, `bN` bet/raise *to* N chips on the current street, `/`
//! between streets; after an all-in call the remaining streets appear as trailing `/`.
//!
//! Seat convention: Slumbot `client_pos` 0 = big blind, 1 = small blind/button. nlhe heads-up:
//! seat 0 = SB/button (acts first preflop), seat 1 = BB (acts first postflop). Hence
//! `nlhe_seat = 1 - client_pos`. Parsing drives a real `nlhe::State`, so legality (min-raise,
//! stacks) is checked by the engine.

use crate::mix;
use crate::policy::{Event, PolicyFactory, View};
use crate::stats::{Acc, Est};
use cards::{parse_card, Card, Rng};
use nlhe::{Action, GameConfig, State, Step};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const STACK: i32 = 20_000;
pub const SMALL_BLIND: i32 = 50;
pub const BIG_BLIND: i32 = 100;

pub fn game_config() -> GameConfig {
    GameConfig {
        players: 2,
        stack: STACK,
        small_blind: SMALL_BLIND,
        big_blind: BIG_BLIND,
        stacks: None,
    }
}

pub fn nlhe_seat(client_pos: usize) -> usize {
    1 - client_pos
}

#[derive(Debug)]
pub struct Parsed {
    pub events: Vec<Event>,
    /// State after all events.
    pub state: State,
}

pub fn parse_actions(s: &str) -> Result<Parsed, String> {
    let cfg = game_config();
    let mut st = State::new(&cfg);
    let b = s.as_bytes();
    let (mut i, mut events) = (0, Vec::new());
    while i < b.len() {
        if st.is_terminal() {
            // Only run-out street separators may follow an all-in call.
            if st.showdown() && b[i..].iter().all(|&c| c == b'/') {
                break;
            }
            return Err(format!("action continues after terminal hand at {i}"));
        }
        let seat = st.to_act().unwrap();
        let l = st.legal();
        let c = b[i] as char;
        i += 1;
        let action = match c {
            'k' if l.can_check => Action::Check,
            'c' if !l.can_check => Action::Call,
            'f' if l.can_fold => Action::Fold,
            'b' => {
                let start = i;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                let t: i32 = s[start..i]
                    .parse()
                    .map_err(|_| "bad bet size".to_string())?;
                if !l.can_raise || t < l.min_raise_to || t > l.max_raise_to {
                    return Err(format!(
                        "illegal bet to {t} (legal {}..{})",
                        l.min_raise_to, l.max_raise_to
                    ));
                }
                Action::RaiseTo(t)
            }
            c => return Err(format!("illegal action '{c}' at {}", i - 1)),
        };
        events.push(Event {
            seat: seat as u8,
            street: st.street(),
            action,
        });
        match st.apply(action) {
            Step::NewStreet(_) => {
                if i < b.len() {
                    if b[i] != b'/' {
                        return Err(format!("missing street separator at {i}"));
                    }
                    i += 1;
                }
            }
            Step::Terminal if !st.showdown() && i != b.len() => {
                return Err("extra characters after fold".into())
            }
            _ => {}
        }
    }
    Ok(Parsed { events, state: st })
}

/// Canonical action string for an event list (inverse of `parse_actions`).
pub fn format_actions(events: &[Event]) -> String {
    let mut st = State::new(&game_config());
    let mut out = String::new();
    let mut last_street = 0;
    for e in events {
        last_street = st.street().idx();
        out.push_str(&to_incr(e.action));
        match st.apply(e.action) {
            Step::NewStreet(_) => out.push('/'),
            Step::Terminal if st.showdown() => (last_street..3).for_each(|_| out.push('/')),
            _ => {}
        }
    }
    let _ = last_street;
    out
}

/// Slumbot `incr` token for an action.
pub fn to_incr(a: Action) -> String {
    match a {
        Action::Fold => "f".into(),
        Action::Check => "k".into(),
        Action::Call => "c".into(),
        Action::RaiseTo(t) => format!("b{t}"),
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SlumbotResponse {
    #[serde(default)]
    pub old_action: String,
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub client_pos: usize,
    #[serde(default)]
    pub hole_cards: Vec<String>,
    #[serde(default)]
    pub board: Vec<String>,
    #[serde(default)]
    pub bot_hole_cards: Vec<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub winnings: Option<i64>,
    #[serde(default)]
    pub error_msg: Option<String>,
}

pub struct SlumbotClient {
    host: String,
}

impl Default for SlumbotClient {
    fn default() -> Self {
        Self::new("https://slumbot.com")
    }
}

impl SlumbotClient {
    pub fn new(host: impl Into<String>) -> Self {
        Self { host: host.into() }
    }
    pub fn new_hand(&self, token: Option<&str>) -> Result<SlumbotResponse, String> {
        self.post(
            "/slumbot/api/new_hand",
            token.map_or(json!({}), |t| json!({ "token": t })),
        )
    }
    pub fn act(&self, token: &str, incr: &str) -> Result<SlumbotResponse, String> {
        self.post("/slumbot/api/act", json!({ "token": token, "incr": incr }))
    }
    fn post(&self, path: &str, body: serde_json::Value) -> Result<SlumbotResponse, String> {
        let url = format!("{}{}", self.host.trim_end_matches('/'), path);
        let r: SlumbotResponse = ureq::post(&url)
            .set("Content-Type", "application/json")
            .send_json(body)
            .map_err(|e| format!("Slumbot request failed: {e}"))?
            .into_json()
            .map_err(|e| format!("Slumbot response not JSON: {e}"))?;
        match &r.error_msg {
            Some(m) => Err(format!("Slumbot error: {m}")),
            None => Ok(r),
        }
    }
}

fn cards_of(raw: &[String]) -> Result<Vec<Card>, String> {
    raw.iter()
        .map(|c| parse_card(c).ok_or(format!("bad card {c}")))
        .collect()
}

#[derive(Debug)]
pub struct LiveResult {
    pub hands: usize,
    pub total_chips: i64,
    pub mbb: Est,
    /// Same, with all-in showdowns replaced by their exact all-in EV ("luck adjusted"; the
    /// opponent is not a known strategy, so full AIVAT is impossible, but this removes the
    /// run-out variance of every hand that went all-in before the river).
    pub mbb_adj: Est,
    pub token: Option<String>,
}

/// Politeness / logging options for [`play_live`].
#[derive(Clone, Debug, Default)]
pub struct LiveOpts {
    /// Minimum milliseconds between two HTTP requests (we also never have two in flight).
    pub pace_ms: u64,
    /// Append one JSON line per finished hand here.
    pub log: Option<std::path::PathBuf>,
}

/// Exact (or, preflop, sampled) all-in EV in chips for our side if the hand ended in a
/// showdown with cards still to come after the last betting action; `None` otherwise.
pub fn allin_ev(resp: &SlumbotResponse) -> Option<f64> {
    use cards::{bit, evaluate, mask_of};
    let parsed = parse_actions(&resp.action).ok()?;
    let last = parsed.events.last()?;
    if !parsed.state.showdown() || last.street == nlhe::Street::River {
        return None;
    }
    let known = last.street.board_len();
    let board = cards_of(&resp.board).ok()?;
    let hole = cards_of(&resp.hole_cards).ok()?;
    let opp = cards_of(&resp.bot_hole_cards).ok()?;
    if hole.len() != 2 || opp.len() != 2 || board.len() < 5 {
        return None;
    }
    let seat = nlhe_seat(resp.client_pos);
    let (contrib, pot) = (parsed.state.contributed(seat) as f64, parsed.state.pot() as f64);
    let b0 = mask_of(&board[..known]);
    let (hm, om) = (bit(hole[0]) | bit(hole[1]), bit(opp[0]) | bit(opp[1]));
    let dead = b0 | hm | om;
    let free: Vec<Card> = cards::cards_in(cards::FULL_DECK & !dead).collect();
    let need = 5 - known;
    let (mut win, mut n) = (0.0, 0.0);
    let mut score = |extra: cards::Hand| {
        let b = b0 | extra;
        let (x, y) = (evaluate(hm | b), evaluate(om | b));
        win += if x > y { 1.0 } else if x == y { 0.5 } else { 0.0 };
        n += 1.0;
    };
    match need {
        1 => free.iter().for_each(|&c| score(bit(c))),
        2 => {
            for i in 0..free.len() {
                for j in i + 1..free.len() {
                    score(bit(free[i]) | bit(free[j]));
                }
            }
        }
        _ => {
            let mut rng = Rng::new(0x5EED ^ resp.hole_cards.len() as u64 ^ bit(hole[0]));
            for _ in 0..30_000 {
                let mut d = dead;
                let mut ex = 0;
                for _ in 0..need {
                    let c = rng.draw(d);
                    d |= bit(c);
                    ex |= bit(c);
                }
                score(ex);
            }
        }
    }
    Some(win / n * pot - contrib)
}

/// Play `hands` live hands. Never called by tests.
pub fn play_live(
    client: &SlumbotClient,
    policy: &dyn PolicyFactory,
    hands: usize,
    mut token: Option<String>,
    seed: u64,
    opts: &LiveOpts,
) -> Result<LiveResult, String> {
    let cfg = game_config();
    let (mut acc, mut acc_adj, mut total) = (Acc::default(), Acc::default(), 0i64);
    let mut last_req = std::time::Instant::now() - std::time::Duration::from_secs(10);
    let pace = std::time::Duration::from_millis(opts.pace_ms);
    // One request at a time, spaced by `pace`, with a few retries on transient errors.
    let mut call = |f: &dyn Fn() -> Result<SlumbotResponse, String>| -> Result<SlumbotResponse, String> {
        let mut err = String::new();
        for attempt in 0..5u32 {
            let wait = pace.saturating_sub(last_req.elapsed());
            std::thread::sleep(wait);
            last_req = std::time::Instant::now();
            match f() {
                Ok(r) => return Ok(r),
                Err(e) if e.contains("Slumbot error") => return Err(e),
                Err(e) => {
                    err = e;
                    std::thread::sleep(std::time::Duration::from_secs(2u64 << attempt));
                }
            }
        }
        Err(err)
    };
    for h in 0..hands {
        let mut resp = call(&|| client.new_hand(token.as_deref()))?;
        token = resp.token.clone().or(token);
        let mut rng = Rng::new(mix(seed, h as u64));
        let mut player = None;
        let (mut fed, mut board_seen) = (0usize, 0usize);
        for step in 0..200 {
            if let Some(w) = resp.winnings {
                total += w;
                acc.add(w as f64);
                let adj = allin_ev(&resp).unwrap_or(w as f64);
                acc_adj.add(adj);
                if let Some(path) = &opts.log {
                    use std::io::Write;
                    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                        let _ = writeln!(
                            f,
                            "{}",
                            json!({"hand": h, "client_pos": resp.client_pos, "action": resp.action,
                                   "hole": resp.hole_cards, "bot_hole": resp.bot_hole_cards,
                                   "board": resp.board, "winnings": w, "ev_adj": adj})
                        );
                    }
                }
                break;
            }
            let seat = nlhe_seat(resp.client_pos);
            let parsed = parse_actions(&resp.action)?;
            if parsed.state.to_act() != Some(seat) {
                return Err(format!(
                    "not our turn: action {:?} client_pos {}",
                    resp.action, resp.client_pos
                ));
            }
            let hole = cards_of(&resp.hole_cards)?;
            let board = cards_of(&resp.board)?;
            if hole.len() != 2 {
                return Err("expected two hole cards".into());
            }
            let p = player
                .get_or_insert_with(|| policy.new_player(seat, &cfg, mix(seed, 7 + h as u64)));
            let mut mirror = State::new(&cfg);
            for (k, e) in parsed.events.iter().enumerate() {
                let pre = mirror;
                mirror.apply(e.action);
                if k >= fed {
                    p.observe(&pre, e.seat as usize, e.action);
                }
            }
            fed = parsed.events.len();
            if board.len() > board_seen {
                p.observe_board(&board);
                board_seen = board.len();
            }
            let view = View {
                cfg: &cfg,
                state: &parsed.state,
                board: &board,
                history: &parsed.events,
                seat,
            };
            let a = p.act(&view, [hole[0], hole[1]], rng.f64());
            let t = token.clone().ok_or("no token")?;
            let incr = to_incr(a);
            resp = call(&|| client.act(&t, &incr))?;
            token = resp.token.clone().or(token);
            if step == 199 {
                return Err("hand exceeded action limit".into());
            }
        }
    }
    Ok(LiveResult {
        hands,
        total_chips: total,
        mbb: Est::from_chips(&acc, BIG_BLIND as f64),
        mbb_adj: Est::from_chips(&acc_adj, BIG_BLIND as f64),
        token,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_recorded_strings() {
        for s in [
            "",
            "c",
            "b300c/kk/kk/kb200f",
            "b200c/kk/kk/kb200",
            "b20000c///",
            "b300c/b900c/kk/kk",
            "ck/kk/kk/kk",
            "b300c/kb500c/kk/kk",
            "cb300b900b3000c/kk/kk/kk",
        ] {
            let p = parse_actions(s).unwrap_or_else(|e| panic!("{s}: {e}"));
            assert_eq!(format_actions(&p.events), s, "roundtrip {s}");
        }
    }

    #[test]
    fn seat_convention_and_progress() {
        // Empty string: SB (slumbot pos 1 = nlhe seat 0) to act.
        let p = parse_actions("").unwrap();
        assert_eq!(p.state.to_act(), Some(nlhe_seat(1)));
        // SB limps, BB checks -> flop, BB (pos 0 = seat 1) acts first.
        let p = parse_actions("ck/").unwrap();
        assert_eq!(p.state.to_act(), Some(nlhe_seat(0)));
        assert_eq!(p.state.street(), nlhe::Street::Flop);
        // Raise-call preflop then bets.
        let p = parse_actions("b300c/kb200").unwrap();
        assert_eq!(p.state.to_act(), Some(nlhe_seat(0)));
        assert_eq!(p.state.pot(), 800);
        let p = parse_actions("b300f").unwrap();
        assert!(p.state.is_terminal() && !p.state.showdown());
        // All-in call -> showdown with run-out slashes.
        let p = parse_actions("b20000c///").unwrap();
        assert!(p.state.showdown());
        let p2 = parse_actions("b20000c").unwrap();
        assert!(p2.state.showdown());
    }

    #[test]
    fn allin_ev_preflop_and_flop() {
        let mk = |action: &str, board: &[&str], hole: &[&str], bot: &[&str]| SlumbotResponse {
            old_action: String::new(),
            action: action.into(),
            client_pos: 1,
            hole_cards: hole.iter().map(|s| s.to_string()).collect(),
            board: board.iter().map(|s| s.to_string()).collect(),
            bot_hole_cards: bot.iter().map(|s| s.to_string()).collect(),
            token: None,
            winnings: Some(0),
            error_msg: None,
        };
        // AA vs KK all-in preflop: ~81.9% -> EV = 0.819 * 40000 - 20000 ~ +12.8k.
        let r = mk("b20000c///", &["2c", "7d", "9h", "Js", "Qc"], &["As", "Ah"], &["Kc", "Kd"]);
        let ev = allin_ev(&r).unwrap();
        assert!((12_000.0..13_500.0).contains(&ev), "{ev}");
        // River showdown: no adjustment (nothing left to come).
        let r = mk("ck/kk/kk/kb100c", &["2c", "7d", "9h", "Js", "Qc"], &["As", "Ah"], &["Kc", "Kd"]);
        assert!(allin_ev(&r).is_none());
        // Flop all-in, set vs flush draw: ahead but not certain, EV between -contrib and +pot share.
        let r = mk("b300c/b19700c//", &["2c", "7d", "9h", "Js", "Qc"], &["9s", "9d"], &["Ac", "Kc"]);
        let ev = allin_ev(&r);
        assert!(ev.is_none() || ev.unwrap().abs() <= 20_000.0);
    }

    #[test]
    fn rejects_illegal_strings() {
        for s in [
            "k",
            "c/",
            "b100",
            "b50",
            "b30000",
            "b300cf",
            "b300c/kk/kk/kk/k",
            "b300c/c",
            "x",
            "ckk",
            "b300f/",
        ] {
            assert!(parse_actions(s).is_err(), "{s} should be illegal");
        }
    }
}
