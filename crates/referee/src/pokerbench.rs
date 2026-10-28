//! PokerBench (RZ412/PokerBench, Apache-2.0): parser, engine replay and policy scoring.
//!
//! Data is treated as untrusted text: it is only split and parsed, never evaluated.
//!
//! # Format (verified on the shipped CSVs)
//! * Preflop `*_game_scenario_information.csv`: `prev_line` is `POS/ACTION/POS/ACTION/...`
//!   with sizes as raise-to totals in bb (`2.0bb`), `allin`, `call`, `fold`, `check`. Players
//!   who never act before the hero are silently folded.
//! * Postflop CSVs are heads-up (`IP`/`OOP`) after a preflop line (`preflop_action`, same
//!   grammar). `postflop_action` is `OOP_CHECK/IP_BET_5/OOP_RAISE_14/.../dealcards/Jc/...`
//!   with integer-rounded raise-to sizes in bb. Options are `['Fold', 'Call', 'Raise 29']`.
//!   Labels are a single action (selected because it has >50% solver frequency).
//! * Stacks are 100bb, pot sizes include all bets (`pot_size`), 6-max positions
//!   UTG,HJ,CO,BTN,SB,BB. We map to our seats (SB=0, BB=1, UTG=2, HJ=3, CO=4, BTN=5).
//!
//! # Scoring
//! The policy's distribution over its own concrete legal actions is mapped onto the spot's
//! solver options: fold/check/call exactly; each raise goes to the solver raise (or all-in)
//! option whose raise increment is nearest in log-ratio (see [`crate::replay::nearest_action`]).
//! A policy raise when the solver offers no raise is "unmapped" (counts as aggressive class,
//! never as exact). Reported per spot:
//! * `exact` - argmax mapped option equals the label (greedy policy accuracy);
//! * `class` - argmax of class mass (fold / check-call / bet-raise) equals the label's class;
//! * `mass_exact` / `mass_class` - probability mass on the label / its class (the expected
//!   accuracy of a policy that samples its distribution).

use crate::replay::{Replay, BB};
use bench::mix;
use bench::policy::PolicyFactory;
use cards::{parse_card, parse_cards, Card};
use nlhe::{Action, Street};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const POS_NAMES: [&str; 6] = ["SB", "BB", "UTG", "HJ", "CO", "BTN"];

/// Seat for a PokerBench position name.
pub fn pos_seat(p: &str) -> Option<usize> {
    POS_NAMES.iter().position(|&n| n == p)
}

/// One action token. Sizes are raise-to totals in bb.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tok {
    Fold,
    Check,
    Call,
    Bet(f64),
    Raise(f64),
    AllIn,
}

impl Tok {
    /// 0 = fold, 1 = check/call, 2 = bet/raise/all-in.
    pub fn class(self) -> usize {
        match self {
            Tok::Fold => 0,
            Tok::Check | Tok::Call => 1,
            _ => 2,
        }
    }
    pub fn size(self) -> Option<f64> {
        match self {
            Tok::Bet(x) | Tok::Raise(x) => Some(x),
            _ => None,
        }
    }
}

pub const CLASS_NAMES: [&str; 3] = ["fold", "check_call", "bet_raise"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Oop,
    Ip,
}

/// One token of the postflop line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PostTok {
    Act(Role, Tok),
    /// `dealcards` marker (followed by the card in the source).
    Deal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hero {
    Seat(usize),
    Role(Role),
}

#[derive(Clone, Debug)]
pub struct Spot {
    pub test: bool,
    pub row: usize,
    pub street: Street,
    /// Preflop line as (seat, token).
    pub pre_line: Vec<(usize, Tok)>,
    pub post_line: Vec<PostTok>,
    /// Full board listed in the row (0 or 3..5 cards; postflop rows list all 5).
    pub board_all: Vec<Card>,
    pub hole: [Card; 2],
    pub hero: Hero,
    pub options: Vec<Tok>,
    pub label: Tok,
    pub pot_bb: f64,
}

// ---------------------------------------------------------------- csv + tokens

/// Minimal RFC-4180 CSV: quotes, doubled quotes, newlines in quotes. Returns rows of fields.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut f = String::new();
    let mut q = false;
    let mut it = text.chars().peekable();
    let mut dirty = false;
    while let Some(c) = it.next() {
        dirty = true;
        if q {
            if c == '"' {
                if it.peek() == Some(&'"') {
                    f.push('"');
                    it.next();
                } else {
                    q = false;
                }
            } else {
                f.push(c);
            }
        } else {
            match c {
                '"' => q = true,
                ',' => row.push(std::mem::take(&mut f)),
                '\r' => {}
                '\n' => {
                    row.push(std::mem::take(&mut f));
                    rows.push(std::mem::take(&mut row));
                    dirty = false;
                }
                _ => f.push(c),
            }
        }
    }
    if dirty {
        row.push(f);
        rows.push(row);
    }
    rows
}

fn parse_bb(s: &str) -> Option<f64> {
    s.strip_suffix("bb")?.parse().ok()
}

/// Preflop token: `fold|call|check|allin|<x>bb`.
pub fn parse_pre_tok(s: &str) -> Option<Tok> {
    match s.to_ascii_lowercase().as_str() {
        "fold" => Some(Tok::Fold),
        "call" => Some(Tok::Call),
        "check" => Some(Tok::Check),
        "allin" => Some(Tok::AllIn),
        x => parse_bb(x).map(Tok::Raise),
    }
}

pub fn fmt_pre_tok(t: Tok) -> String {
    match t {
        Tok::Fold => "fold".into(),
        Tok::Call => "call".into(),
        Tok::Check => "check".into(),
        Tok::AllIn => "allin".into(),
        Tok::Bet(x) | Tok::Raise(x) => format!("{x:.1}bb"),
    }
}

pub fn parse_pre_line(s: &str) -> Result<Vec<(usize, Tok)>, String> {
    if s.is_empty() {
        return Ok(vec![]);
    }
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() % 2 != 0 {
        return Err(format!("odd preflop line {s}"));
    }
    parts
        .chunks(2)
        .map(|c| {
            let seat = pos_seat(c[0]).ok_or(format!("bad position {}", c[0]))?;
            let t = parse_pre_tok(c[1]).ok_or(format!("bad token {}", c[1]))?;
            Ok((seat, t))
        })
        .collect()
}

pub fn fmt_pre_line(l: &[(usize, Tok)]) -> String {
    l.iter()
        .map(|&(s, t)| format!("{}/{}", POS_NAMES[s], fmt_pre_tok(t)))
        .collect::<Vec<_>>()
        .join("/")
}

pub fn parse_post_line(s: &str) -> Result<Vec<PostTok>, String> {
    let mut out = Vec::new();
    let mut skip_card = false;
    for t in s.split('/').filter(|t| !t.is_empty()) {
        if skip_card {
            skip_card = false;
            parse_card(t).ok_or(format!("bad dealt card {t}"))?;
            continue;
        }
        if t == "dealcards" {
            out.push(PostTok::Deal);
            skip_card = true;
            continue;
        }
        let (role, rest) = if let Some(r) = t.strip_prefix("OOP_") {
            (Role::Oop, r)
        } else if let Some(r) = t.strip_prefix("IP_") {
            (Role::Ip, r)
        } else {
            return Err(format!("bad postflop token {t}"));
        };
        let mut it = rest.split('_');
        let verb = it.next().unwrap_or("");
        let num = it.next().and_then(|x| x.parse::<f64>().ok());
        let tok = match (verb, num) {
            ("CHECK", None) => Tok::Check,
            ("CALL", None) => Tok::Call,
            ("FOLD", None) => Tok::Fold,
            ("ALLIN", _) => Tok::AllIn,
            ("BET", Some(x)) => Tok::Bet(x),
            ("RAISE", Some(x)) => Tok::Raise(x),
            _ => return Err(format!("bad postflop token {t}")),
        };
        out.push(PostTok::Act(role, tok));
    }
    Ok(out)
}

/// Inverse of [`parse_post_line`] given the dealt cards (turn, river).
pub fn fmt_post_line(l: &[PostTok], dealt: &[Card]) -> String {
    let mut d = 0;
    l.iter()
        .map(|t| match *t {
            PostTok::Deal => {
                let c = dealt
                    .get(d)
                    .map_or("??".to_string(), |&c| cards::card_str(c));
                d += 1;
                format!("dealcards/{c}")
            }
            PostTok::Act(r, tok) => {
                let r = if r == Role::Oop { "OOP" } else { "IP" };
                match tok {
                    Tok::Check => format!("{r}_CHECK"),
                    Tok::Call => format!("{r}_CALL"),
                    Tok::Fold => format!("{r}_FOLD"),
                    Tok::AllIn => format!("{r}_ALLIN"),
                    Tok::Bet(x) => format!("{r}_BET_{x}"),
                    Tok::Raise(x) => format!("{r}_RAISE_{x}"),
                }
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Postflop option / label: `Check|Call|Fold|Bet 24|Raise 29|Allin`.
pub fn parse_post_option(s: &str) -> Option<Tok> {
    let s = s.trim();
    let mut it = s.split_whitespace();
    let verb = it.next()?.to_ascii_lowercase();
    let num = it.next().and_then(|x| x.parse::<f64>().ok());
    match (verb.as_str(), num) {
        ("check", None) => Some(Tok::Check),
        ("call", None) => Some(Tok::Call),
        ("fold", None) => Some(Tok::Fold),
        ("allin", _) => Some(Tok::AllIn),
        ("bet", Some(x)) => Some(Tok::Bet(x)),
        ("raise", Some(x)) => Some(Tok::Raise(x)),
        _ => None,
    }
}

/// `['a', 'b c']` -> items.
pub fn parse_list(s: &str) -> Vec<String> {
    s.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|x| x.trim().trim_matches('\'').trim_matches('"').to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

fn col(h: &[String], name: &str) -> Result<usize, String> {
    h.iter()
        .position(|x| x == name)
        .ok_or(format!("missing column {name}"))
}

fn hole_of(s: &str) -> Result<[Card; 2], String> {
    let c = parse_cards(s).ok_or(format!("bad holding {s}"))?;
    if c.len() != 2 || c[0] == c[1] {
        return Err(format!("bad holding {s}"));
    }
    Ok([c[0], c[1]])
}

/// Parse a preflop CSV (header row first).
pub fn parse_preflop_csv(text: &str, test: bool) -> Vec<Result<Spot, String>> {
    let rows = parse_csv(text);
    let Some((h, body)) = rows.split_first() else {
        return vec![];
    };
    let (cl, cp, ch, cd, co, cs) = match (|| {
        Ok::<_, String>((
            col(h, "prev_line")?,
            col(h, "hero_pos")?,
            col(h, "hero_holding")?,
            col(h, "correct_decision")?,
            col(h, "available_moves")?,
            col(h, "pot_size")?,
        ))
    })() {
        Ok(x) => x,
        Err(e) => return vec![Err(e)],
    };
    body.iter()
        .enumerate()
        .map(|(row, r)| {
            let g = |i: usize| r.get(i).map(|s| s.as_str()).ok_or("short row".to_string());
            let hero = pos_seat(g(cp)?).ok_or("bad hero_pos")?;
            let options = parse_list(g(co)?)
                .iter()
                .map(|x| parse_pre_tok(x).ok_or(format!("bad option {x}")))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Spot {
                test,
                row,
                street: Street::Preflop,
                pre_line: parse_pre_line(g(cl)?)?,
                post_line: vec![],
                board_all: vec![],
                hole: hole_of(g(ch)?)?,
                hero: Hero::Seat(hero),
                options,
                label: parse_pre_tok(g(cd)?).ok_or("bad label")?,
                pot_bb: g(cs)?.parse().map_err(|_| "bad pot")?,
            })
        })
        .collect()
}

/// Parse a postflop CSV (header row first).
pub fn parse_postflop_csv(text: &str, test: bool) -> Vec<Result<Spot, String>> {
    let rows = parse_csv(text);
    let Some((h, body)) = rows.split_first() else {
        return vec![];
    };
    let cols = (|| {
        Ok::<_, String>([
            col(h, "preflop_action")?,
            col(h, "board_flop")?,
            col(h, "board_turn")?,
            col(h, "board_river")?,
            col(h, "postflop_action")?,
            col(h, "evaluation_at")?,
            col(h, "available_moves")?,
            col(h, "pot_size")?,
            col(h, "hero_position")?,
            col(h, "holding")?,
            col(h, "correct_decision")?,
        ])
    })();
    let c = match cols {
        Ok(c) => c,
        Err(e) => return vec![Err(e)],
    };
    body.iter()
        .enumerate()
        .map(|(row, r)| {
            let g = |i: usize| {
                r.get(c[i])
                    .map(|s| s.as_str())
                    .ok_or("short row".to_string())
            };
            let mut board = parse_cards(g(1)?).ok_or("bad flop")?;
            board.extend(parse_cards(g(2)?).ok_or("bad turn")?);
            board.extend(parse_cards(g(3)?).ok_or("bad river")?);
            let street = match g(5)? {
                "Flop" => Street::Flop,
                "Turn" => Street::Turn,
                "River" => Street::River,
                x => return Err(format!("bad street {x}")),
            };
            if board.len() != 5 {
                return Err("board is not 5 cards".into());
            }
            let hero = match g(8)? {
                "IP" => Role::Ip,
                "OOP" => Role::Oop,
                x => return Err(format!("bad hero_position {x}")),
            };
            let options = parse_list(g(6)?)
                .iter()
                .map(|x| parse_post_option(x).ok_or(format!("bad option {x}")))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Spot {
                test,
                row,
                street,
                pre_line: parse_pre_line(g(0)?)?,
                post_line: parse_post_line(g(4)?)?,
                board_all: board,
                hole: hole_of(g(9)?)?,
                hero: Hero::Role(hero),
                options,
                label: parse_post_option(g(10)?).ok_or("bad label")?,
                pot_bb: g(7)?.parse().map_err(|_| "bad pot")?,
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Preflop,
    Postflop,
    All,
}

/// Dataset file name for (kind, split).
pub fn file_name(pre: bool, test: bool) -> &'static str {
    match (pre, test) {
        (true, true) => "preflop_1k_test_set_game_scenario_information.csv",
        (true, false) => "preflop_60k_train_set_game_scenario_information.csv",
        (false, true) => "postflop_10k_test_set_game_scenario_information.csv",
        (false, false) => "postflop_500k_train_set_game_scenario_information.csv",
    }
}

/// Default data directory (`<workspace>/data/pokerbench`).
pub fn default_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/pokerbench")
}

/// Stream a split in chunks of at most `chunk` rows (memory stays bounded: the file text is
/// read once, rows are parsed chunk by chunk). `limit > 0` first picks an evenly strided
/// subset of rows. `f` receives each chunk of parsed spots (with global row numbers) and the
/// number of rows in the chunk that failed to parse. Assumes no newlines inside fields
/// (true for the shipped files).
pub fn for_each_chunk(
    dir: &Path,
    which: Which,
    test: bool,
    limit: usize,
    chunk: usize,
    mut f: impl FnMut(Vec<Spot>, usize),
) -> Result<(), String> {
    // With `Which::All` and a limit the budget is split proportionally to file sizes.
    let files: Vec<bool> = [true, false]
        .into_iter()
        .filter(|&pre| !((pre && which == Which::Postflop) || (!pre && which == Which::Preflop)))
        .collect();
    let mut texts = Vec::new();
    for &pre in &files {
        let p = dir.join(file_name(pre, test));
        texts.push((
            pre,
            std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?,
        ));
    }
    let total_rows: usize = texts
        .iter()
        .map(|t| t.1.lines().count().saturating_sub(1))
        .sum();
    for (pre, text) in &texts {
        let mut lines = text.lines();
        let header = lines.next().ok_or("empty file")?;
        let body: Vec<&str> = lines.collect();
        let k = if limit == 0 || limit >= total_rows {
            body.len()
        } else {
            (limit * body.len() / total_rows.max(1)).max(1)
        };
        let idx: Vec<usize> = if k >= body.len() {
            (0..body.len()).collect()
        } else {
            (0..k).map(|i| i * body.len() / k).collect()
        };
        for part in idx.chunks(chunk.max(1)) {
            let mut t = String::from(header);
            for &i in part {
                t.push('\n');
                t.push_str(body[i]);
            }
            let parsed = if *pre {
                parse_preflop_csv(&t, test)
            } else {
                parse_postflop_csv(&t, test)
            };
            let (mut ok, mut bad) = (Vec::new(), 0);
            for (r, &i) in parsed.into_iter().zip(part) {
                match r {
                    Ok(mut s) => {
                        s.row = i;
                        ok.push(s)
                    }
                    Err(_) => bad += 1,
                }
            }
            f(ok, bad);
        }
    }
    Ok(())
}

/// Load a split fully (use [`for_each_chunk`] for the large train files).
pub fn load(dir: &Path, which: Which, test: bool) -> Result<(Vec<Spot>, usize), String> {
    let (mut spots, mut bad) = (Vec::new(), 0);
    for_each_chunk(dir, which, test, 0, 20_000, |s, b| {
        spots.extend(s);
        bad += b;
    })?;
    Ok((spots, bad))
}

/// Deterministic evenly strided subsample of at most `n` items.
pub fn stride<T: Clone>(v: &[T], n: usize) -> Vec<T> {
    if n == 0 || n >= v.len() {
        return v.to_vec();
    }
    (0..n).map(|i| v[i * v.len() / n].clone()).collect()
}

// ---------------------------------------------------------------- replay

pub struct Replayed {
    pub replay: Replay,
    pub hero_seat: usize,
    pub pot_diff_bb: f64,
}

fn to_action(r: &Replay, t: Tok) -> Result<Action, String> {
    let l = r.state.legal();
    match t {
        Tok::Fold => Ok(Action::Fold),
        Tok::Check => Ok(Action::Check),
        Tok::Call => Ok(if l.can_check {
            Action::Check
        } else {
            Action::Call
        }),
        Tok::Bet(x) | Tok::Raise(x) => r.snap_raise((x * BB as f64).round() as i32),
        Tok::AllIn => Ok(if l.can_raise {
            Action::RaiseTo(l.max_raise_to)
        } else {
            Action::Call
        }),
    }
}

/// Replay the preflop line (folding skipped seats), up to the point where `until_hero` (a
/// seat) is to act, or until the street changes when `until_hero` is `None`.
fn replay_preflop(
    r: &mut Replay,
    line: &[(usize, Tok)],
    hero: Option<usize>,
) -> Result<(), String> {
    for &(seat, t) in line {
        if r.state.street() != Street::Preflop {
            return Err("street advanced mid-line".into());
        }
        r.skip_to(seat)?;
        let a = to_action(r, t)?;
        r.apply(a)?;
    }
    if let Some(h) = hero {
        r.skip_to(h)?;
        if r.state.street() != Street::Preflop {
            return Err("street advanced".into());
        }
    } else {
        // Heads-up postflop spots: everyone still pending when the line ends folded.
        while r.state.street() == Street::Preflop && r.to_act().is_some() {
            r.apply(Action::Fold)?;
        }
    }
    Ok(())
}

/// Check option consistency with the engine's legal actions at the hero decision. The source
/// lists `Fold` next to `Check` and omits `Call` for some prunable nodes, so only options that
/// are listed but impossible are errors.
fn check_options(r: &Replay, opts: &[Tok]) -> Result<(), String> {
    let l = r.state.legal();
    let has = |f: fn(&Tok) -> bool| opts.iter().any(f);
    if has(|t| *t == Tok::Fold) && !(l.can_fold || l.can_check) {
        return Err("fold option mismatch".into());
    }
    if has(|t| *t == Tok::Check) && !l.can_check {
        return Err("check option mismatch".into());
    }
    if has(|t| *t == Tok::Call) && l.can_check {
        return Err("call option mismatch".into());
    }
    if !l.can_raise && has(|t| t.class() == 2) {
        return Err("raise option but cannot raise".into());
    }
    Ok(())
}

/// Replay `spot` to the hero's decision. `full_board`: expose all five board cards to the
/// replay (used by river referee); otherwise only cards up to the hero's street are known.
pub fn replay_spot(spot: &Spot) -> Result<Replayed, String> {
    let known = spot.street.board_len();
    let board: Vec<Card> = spot.board_all.iter().take(known).copied().collect();
    if spot.hole.iter().any(|h| board.contains(h)) {
        return Err("hole collides with board".into());
    }
    let mut r = Replay::new(6, &board);
    let hero_seat;
    match spot.hero {
        Hero::Seat(h) => {
            hero_seat = h;
            replay_preflop(&mut r, &spot.pre_line, Some(h))?;
        }
        Hero::Role(role) => {
            replay_preflop(&mut r, &spot.pre_line, None)?;
            if r.state.street() != Street::Flop || r.state.live_count() != 2 {
                return Err("preflop line does not end heads-up on the flop".into());
            }
            let (oop, ip) = live_pair(&r);
            let seat_of = |ro: Role| if ro == Role::Oop { oop } else { ip };
            hero_seat = seat_of(role);
            let mut deals = 0;
            for t in &spot.post_line {
                match *t {
                    PostTok::Deal => {
                        deals += 1;
                        if r.state.street().idx() != 1 + deals {
                            return Err("dealcards marker without street change".into());
                        }
                    }
                    PostTok::Act(ro, tok) => {
                        if r.to_act() != Some(seat_of(ro)) {
                            return Err("turn order mismatch".into());
                        }
                        let a = to_action(&r, tok)?;
                        r.apply(a)?;
                    }
                }
            }
            if r.state.street() != spot.street {
                return Err("wrong street at hero".into());
            }
            if r.to_act() != Some(hero_seat) {
                return Err("hero not to act".into());
            }
        }
    }
    if r.state.is_terminal() || r.to_act() != Some(hero_seat) {
        return Err("hero not to act".into());
    }
    check_options(&r, &spot.options)?;
    let pot_diff_bb = r.state.pot() as f64 / BB as f64 - spot.pot_bb;
    Ok(Replayed {
        replay: r,
        hero_seat,
        pot_diff_bb,
    })
}

/// Replay a postflop spot's history up to the start of the river decision (both players'
/// hole cards unknown). Returns the replay (full board known) and `[oop, ip]` seats.
pub fn replay_to_river(spot: &Spot) -> Result<(Replay, [usize; 2]), String> {
    if !matches!(spot.hero, Hero::Role(_)) || spot.street != Street::River {
        return Err("not a river spot".into());
    }
    let mut r = Replay::new(6, &spot.board_all);
    replay_preflop(&mut r, &spot.pre_line, None)?;
    if r.state.street() != Street::Flop || r.state.live_count() != 2 {
        return Err("preflop line does not end heads-up on the flop".into());
    }
    let (oop, ip) = live_pair(&r);
    let mut deals = 0;
    for t in &spot.post_line {
        match *t {
            PostTok::Deal => {
                deals += 1;
                if r.state.street().idx() != 1 + deals {
                    return Err("dealcards marker without street change".into());
                }
                if deals == 2 {
                    break;
                }
            }
            PostTok::Act(ro, tok) => {
                let seat = if ro == Role::Oop { oop } else { ip };
                if r.to_act() != Some(seat) {
                    return Err("turn order mismatch".into());
                }
                let a = to_action(&r, tok)?;
                r.apply(a)?;
            }
        }
    }
    if r.state.street() != Street::River || r.state.is_terminal() || r.to_act().is_none() {
        return Err("no river decision".into());
    }
    Ok((r, [oop, ip]))
}

/// (OOP seat, IP seat) of the two live players after the preflop line.
pub fn live_pair(r: &Replay) -> (usize, usize) {
    let live: Vec<usize> = (0..6).filter(|&s| !r.state.is_folded(s)).collect();
    (live[0], live[1])
}

// ---------------------------------------------------------------- scoring

#[derive(Clone, Debug, Default, Serialize)]
pub struct Agg {
    pub n: usize,
    pub exact: f64,
    pub class: f64,
    pub mass_exact: f64,
    pub mass_class: f64,
}

impl Agg {
    fn add(&mut self, r: &SpotScore) {
        self.n += 1;
        self.exact += r.exact as u8 as f64;
        self.class += r.class as u8 as f64;
        self.mass_exact += r.mass_exact;
        self.mass_class += r.mass_class;
    }
    fn finish(mut self) -> Agg {
        let n = self.n.max(1) as f64;
        self.exact /= n;
        self.class /= n;
        self.mass_exact /= n;
        self.mass_class /= n;
        self
    }
}

#[derive(Clone, Debug)]
pub struct SpotScore {
    pub exact: bool,
    pub class: bool,
    pub mass_exact: f64,
    pub mass_class: f64,
    pub street: Street,
    pub position: String,
    pub spot_type: String,
    pub label_class: usize,
    pub pot_ok: bool,
}

/// Concrete engine action for a solver option at the replayed state.
fn option_action(r: &Replay, t: Tok) -> Action {
    to_action(r, t).unwrap_or_else(|_| {
        let l = r.state.legal();
        match t {
            Tok::Bet(x) | Tok::Raise(x) => Action::RaiseTo(
                ((x * BB as f64).round() as i32).clamp(l.min_raise_to, l.max_raise_to),
            ),
            _ => Action::Call,
        }
    })
}

fn spot_type(spot: &Spot) -> String {
    match spot.street {
        Street::Preflop => {
            let raises = spot.pre_line.iter().filter(|x| x.1.class() == 2).count();
            let t = match raises {
                0 if spot.pre_line.iter().any(|x| x.1 == Tok::Call) => "limped",
                0 => "unopened",
                1 => "vs_open",
                2 => "vs_3bet",
                3 => "vs_4bet",
                _ => "vs_5bet+",
            };
            let allin = spot.options.contains(&Tok::AllIn) && spot.options.len() <= 3;
            format!("{t}{}", if allin { "_shove_node" } else { "" })
        }
        _ => {
            let shape = if spot.options.contains(&Tok::Fold) {
                if spot.options.iter().any(|t| t.class() == 2) {
                    "facing_bet_can_raise"
                } else {
                    "facing_bet_no_raise"
                }
            } else if spot.options.iter().any(|t| matches!(t, Tok::Raise(_))) {
                "check_or_raise"
            } else {
                "check_or_bet"
            };
            let raises = spot.pre_line.iter().filter(|x| x.1.class() == 2).count();
            let pot = if raises <= 1 { "srp" } else { "3bp+" };
            format!("{shape}/{pot}")
        }
    }
}

/// Score one spot. `seed` seeds the policy's hidden randomness.
pub fn score_spot(spot: &Spot, f: &dyn PolicyFactory, seed: u64) -> Result<SpotScore, String> {
    let rp = replay_spot(spot)?;
    let r = &rp.replay;
    let li = spot
        .options
        .iter()
        .position(|&t| t == spot.label)
        .ok_or("label not among options")?;
    let targets: Vec<Action> = spot.options.iter().map(|&t| option_action(r, t)).collect();
    let mut p = r.player(f, rp.hero_seat, seed);
    let d = r.dist(&mut *p, spot.hole);
    let n = spot.options.len();
    // masses: options 0..n, then unmapped (index n).
    let mut m = vec![0.0f64; n + 1];
    let mut cm = [0.0f64; 3];
    for i in 0..d.len() {
        let (a, pr) = (d.action(i), d.prob(i));
        let cl = match a {
            Action::Fold => 0,
            Action::Check | Action::Call => 1,
            Action::RaiseTo(_) => 2,
        };
        cm[cl] += pr;
        m[crate::replay::nearest_action(&r.state, &targets, a).unwrap_or(n)] += pr;
    }
    let argmax = |v: &[f64]| {
        v.iter()
            .enumerate()
            .fold(
                (0, f64::MIN),
                |b, (i, &x)| if x > b.1 + 1e-12 { (i, x) } else { b },
            )
            .0
    };
    let lc = spot.label.class();
    Ok(SpotScore {
        exact: argmax(&m) == li,
        class: argmax(&cm) == lc,
        mass_exact: m[li],
        mass_class: cm[lc],
        street: spot.street,
        position: match spot.hero {
            Hero::Seat(s) => POS_NAMES[s].to_string(),
            Hero::Role(Role::Ip) => "IP".into(),
            Hero::Role(Role::Oop) => "OOP".into(),
        },
        spot_type: spot_type(spot),
        label_class: lc,
        pot_ok: rp.pot_diff_bb.abs() <= 1.0 + 0.03 * spot.pot_bb,
    })
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub policy: String,
    pub spots_total: usize,
    pub parse_failures: usize,
    pub replayable: usize,
    pub coverage: f64,
    pub pot_matches_source: f64,
    pub failure_reasons: BTreeMap<String, usize>,
    pub overall: Agg,
    pub by_street: BTreeMap<String, Agg>,
    pub by_position: BTreeMap<String, Agg>,
    pub by_spot_type: BTreeMap<String, Agg>,
    pub by_label_class: BTreeMap<String, Agg>,
}

/// Score `spots` in parallel. Seeds are per spot: `mix(seed, row-id)`.
pub fn score_chunk(
    spots: &[Spot],
    f: &dyn PolicyFactory,
    seed: u64,
) -> Vec<Result<SpotScore, String>> {
    spots
        .par_iter()
        .map(|s| {
            let id =
                (s.row as u64) << 2 | (s.street != Street::Preflop) as u64 | (s.test as u64) << 1;
            score_spot(s, f, mix(seed, id))
        })
        .collect()
}

/// Convenience: score and report in one go.
pub fn run(spots: &[Spot], parse_failures: usize, f: &dyn PolicyFactory, seed: u64) -> Report {
    report(&f.name(), parse_failures, score_chunk(spots, f, seed))
}

/// Aggregate per-spot scores into a [`Report`].
pub fn report(name: &str, parse_failures: usize, res: Vec<Result<SpotScore, String>>) -> Report {
    let mut rep = Report {
        policy: name.to_string(),
        spots_total: res.len() + parse_failures,
        parse_failures,
        replayable: 0,
        coverage: 0.0,
        pot_matches_source: 0.0,
        failure_reasons: BTreeMap::new(),
        overall: Agg::default(),
        by_street: BTreeMap::new(),
        by_position: BTreeMap::new(),
        by_spot_type: BTreeMap::new(),
        by_label_class: BTreeMap::new(),
    };
    let mut pot_ok = 0;
    for r in &res {
        match r {
            Err(e) => *rep.failure_reasons.entry(e.clone()).or_default() += 1,
            Ok(s) => {
                rep.replayable += 1;
                pot_ok += s.pot_ok as usize;
                rep.overall.add(s);
                let st = format!("{:?}", s.street).to_lowercase();
                rep.by_street.entry(st).or_default().add(s);
                rep.by_position
                    .entry(s.position.clone())
                    .or_default()
                    .add(s);
                rep.by_spot_type
                    .entry(format!(
                        "{}:{}",
                        if s.street == Street::Preflop {
                            "pre"
                        } else {
                            "post"
                        },
                        s.spot_type
                    ))
                    .or_default()
                    .add(s);
                rep.by_label_class
                    .entry(CLASS_NAMES[s.label_class].to_string())
                    .or_default()
                    .add(s);
            }
        }
    }
    rep.coverage = rep.replayable as f64 / rep.spots_total.max(1) as f64;
    rep.pot_matches_source = pot_ok as f64 / rep.replayable.max(1) as f64;
    rep.overall = std::mem::take(&mut rep.overall).finish();
    for m in [
        &mut rep.by_street,
        &mut rep.by_position,
        &mut rep.by_spot_type,
        &mut rep.by_label_class,
    ] {
        for v in m.values_mut() {
            *v = std::mem::take(v).finish();
        }
    }
    rep
}
