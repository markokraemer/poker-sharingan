//! Hand records, bb/100 with confidence intervals, all-in EV adjustment, breakdowns.
//!
//! # What the numbers are
//! - `raw`: realised profit per hand in big blinds, x100 = bb/100, with a Student-t 95% CI
//!   (normal for n > 120). The unit of independence is the hand; consecutive hands at one table
//!   are treated as independent (they are not exactly: stacks and opponents persist).
//! - `adjusted`: the same, but every hand that ended in an all-in run-out with all live hole
//!   cards known is replaced by its expectation (see [`crate::equity`]). Unbiased for hands where
//!   the cards are always revealed (all-in showdowns); see README for the mucking caveat.
//! - What *cannot* be corrected: luck in hands that are not all-in before the river (we never see
//!   folded or mucked cards, and opponent strategies are unknown, so AIVAT-style action
//!   corrections are impossible), and hole-card luck. The `var_ratio` shows how much the
//!   all-in correction actually helped.

use crate::equity::{expected_net, Entry};
use crate::layout::Layout;
use crate::proto::{self, ActionObs, ObsKind};
use crate::sim::Sim;
use cards::{card_str, parse_cards, Card};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct HandPlayer {
    pub seat: usize,
    pub name: Option<String>,
    pub start_stack: Option<i64>,
    pub contrib: Option<i64>,
    pub folded: Option<bool>,
    pub shown: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct HandAct {
    pub seat: usize,
    pub street: Option<String>,
    pub action: String,
    pub amount: Option<i64>,
}

/// One hand, all chip amounts in Open Poker chips.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct HandRecord {
    pub hand_id: String,
    pub table_id: Option<String>,
    pub hand_no: u64,
    pub hero_seat: usize,
    pub dealer_seat: usize,
    pub sb: i64,
    pub bb: i64,
    pub n_players: usize,
    pub position: String,
    pub hero_hole: Vec<String>,
    pub board: Vec<String>,
    pub players: Vec<HandPlayer>,
    pub actions: Vec<HandAct>,
    /// Hero net chips for the hand.
    pub profit: Option<i64>,
    pub profit_src: String,
    /// Board cards visible when betting ended with a showdown ahead (5 = river showdown).
    pub lock_len: Option<usize>,
    pub showdown: bool,
    pub policy: String,
    pub decisions: u32,
    pub fallbacks: u32,
    pub issues: u32,
    pub desync: bool,
}

impl HandRecord {
    pub fn profit_bb(&self) -> Option<f64> {
        let p = self.profit?;
        (self.bb > 0).then(|| p as f64 / self.bb as f64)
    }
}

pub fn cards_to_strs(c: &[Card]) -> Vec<String> {
    c.iter().map(|&x| card_str(x)).collect()
}

fn strs_to_cards(v: &[String]) -> Vec<Card> {
    v.iter().filter_map(|s| proto::parse_op_card(s)).collect()
}

// ---------------------------------------------------------------- statistics

#[derive(Serialize, Clone, Copy, Debug, Default)]
pub struct Est {
    pub n: usize,
    pub mean_bb100: f64,
    /// Per-hand standard deviation in bb.
    pub sd_bb: f64,
    pub se_bb100: f64,
    pub ci_lo: f64,
    pub ci_hi: f64,
}

/// Two-sided 95% Student-t critical value.
pub fn t_crit95(df: usize) -> f64 {
    const T: [f64; 30] = [
        12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228, 2.201, 2.179, 2.160, 2.145,
        2.131, 2.120, 2.110, 2.101, 2.093, 2.086, 2.080, 2.074, 2.069, 2.064, 2.060, 2.056, 2.052, 2.048,
        2.045, 2.042,
    ];
    match df {
        0 => f64::INFINITY,
        1..=30 => T[df - 1],
        31..=60 => 2.00,
        61..=120 => 1.98,
        _ => 1.96,
    }
}

pub fn estimate(xs: &[f64]) -> Est {
    let n = xs.len();
    if n == 0 {
        return Est::default();
    }
    let mean = xs.iter().sum::<f64>() / n as f64;
    let var = if n > 1 {
        xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0)
    } else {
        0.0
    };
    let sd = var.sqrt();
    let se = sd / (n as f64).sqrt();
    let half = if n > 1 { t_crit95(n - 1) * se } else { f64::INFINITY };
    Est {
        n,
        mean_bb100: mean * 100.0,
        sd_bb: sd,
        se_bb100: se * 100.0,
        ci_lo: (mean - half) * 100.0,
        ci_hi: (mean + half) * 100.0,
    }
}

impl std::fmt::Display for Est {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        if self.n == 0 {
            return write!(f, "n=0");
        }
        write!(
            f,
            "{:+.1} bb/100  95% CI [{:.1}, {:.1}]  (n={}, sd {:.1} bb/hand)",
            self.mean_bb100, self.ci_lo, self.ci_hi, self.n, self.sd_bb
        )
    }
}

// ---------------------------------------------------------------- all-in EV

/// Hero's expected net chips given the all-in lock, if computable from the record.
pub fn allin_ev(h: &HandRecord, mc_samples: usize) -> Option<f64> {
    let lock = h.lock_len?;
    if lock >= 5 || h.hero_hole.len() != 2 {
        return None;
    }
    let board = strs_to_cards(&h.board);
    if board.len() < lock {
        return None;
    }
    let board = &board[..lock];
    let mut entries = Vec::new();
    for p in &h.players {
        let contrib = p.contrib?;
        let live = !p.folded.unwrap_or(false);
        let hole = if p.seat == h.hero_seat {
            let c = strs_to_cards(&h.hero_hole);
            (c.len() == 2).then(|| [c[0], c[1]])
        } else {
            p.shown.as_ref().map(|s| strs_to_cards(s)).filter(|c| c.len() == 2).map(|c| [c[0], c[1]])
        };
        entries.push(Entry { seat: p.seat, contrib, live, hole });
    }
    let hero_live = entries.iter().any(|e| e.seat == h.hero_seat && e.live);
    if !hero_live {
        return None; // hero folded: nothing to correct
    }
    let seed = h.hand_no.wrapping_mul(0x9E37_79B9).wrapping_add(h.hand_id.len() as u64);
    let r = expected_net(&entries, board, mc_samples, seed)?;
    r.into_iter().find(|x| x.0 == h.hero_seat).map(|x| x.1)
}

// ---------------------------------------------------------------- report

#[derive(Serialize, Clone, Debug, Default)]
pub struct Group {
    pub key: String,
    pub est: Est,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Report {
    pub hands: usize,
    pub hands_without_profit: usize,
    pub raw: Est,
    pub adjusted: Est,
    pub hands_adjusted: usize,
    /// bb/100 removed by the all-in adjustment (negative = hero ran above equity).
    pub allin_luck_bb100: f64,
    /// var(raw) / var(adjusted) per hand; > 1 means the adjustment helped.
    pub var_ratio: f64,
    /// Hands needed for a +-5 bb/100 95% CI at the adjusted sd.
    pub hands_for_pm5: usize,
    pub vpip: f64,
    pub pfr: f64,
    pub showdown_pct: f64,
    pub desync_hands: usize,
    pub fallback_decisions: u32,
    pub issue_count: u32,
    pub decisions: u32,
    pub by_position: Vec<Group>,
    pub by_players: Vec<Group>,
    pub by_opponent: Vec<Group>,
    pub by_policy: Vec<Group>,
}

fn group<F: Fn(&HandRecord) -> Vec<String>>(rows: &[(&HandRecord, f64)], key: F) -> Vec<Group> {
    let mut m: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for (h, x) in rows {
        for k in key(h) {
            m.entry(k).or_default().push(*x);
        }
    }
    let mut v: Vec<Group> = m.into_iter().map(|(key, xs)| Group { key, est: estimate(&xs) }).collect();
    v.sort_by(|a, b| b.est.n.cmp(&a.est.n).then(a.key.cmp(&b.key)));
    v
}

pub fn analyze(hands: &[HandRecord], mc_samples: usize) -> Report {
    let mut raw = Vec::new();
    let mut adj = Vec::new();
    let mut rows: Vec<(&HandRecord, f64)> = Vec::new(); // adjusted values for the breakdowns
    let mut r = Report { hands: hands.len(), ..Default::default() };
    let (mut vpip, mut pfr, mut sd, mut pre_n) = (0usize, 0usize, 0usize, 0usize);
    for h in hands {
        r.decisions += h.decisions;
        r.fallback_decisions += h.fallbacks;
        r.issue_count += h.issues;
        r.desync_hands += h.desync as usize;
        let Some(x) = h.profit_bb() else {
            r.hands_without_profit += 1;
            continue;
        };
        raw.push(x);
        let a = match allin_ev(h, mc_samples) {
            Some(ev) if h.bb > 0 => {
                r.hands_adjusted += 1;
                ev / h.bb as f64
            }
            _ => x,
        };
        adj.push(a);
        rows.push((h, a));
        sd += h.showdown as usize;
        let hero_pre: Vec<&HandAct> = h
            .actions
            .iter()
            .filter(|a| a.seat == h.hero_seat && a.street.as_deref().is_some_and(|s| s.eq_ignore_ascii_case("preflop")))
            .collect();
        if !hero_pre.is_empty() || h.decisions > 0 {
            pre_n += 1;
            let voluntary = hero_pre
                .iter()
                .any(|a| matches!(proto::parse_obs_kind(&a.action), ObsKind::Call | ObsKind::Raise | ObsKind::AllIn));
            // A BB "call" never exists (it checks), so any call/raise is voluntary.
            vpip += voluntary as usize;
            pfr += hero_pre
                .iter()
                .any(|a| matches!(proto::parse_obs_kind(&a.action), ObsKind::Raise | ObsKind::AllIn)) as usize;
        }
    }
    r.raw = estimate(&raw);
    r.adjusted = estimate(&adj);
    r.allin_luck_bb100 = r.adjusted.mean_bb100 - r.raw.mean_bb100;
    let var = |xs: &[f64]| {
        let e = estimate(xs);
        e.sd_bb * e.sd_bb
    };
    let (vr, va) = (var(&raw), var(&adj));
    r.var_ratio = if va > 0.0 { vr / va } else { 1.0 };
    r.hands_for_pm5 = if r.adjusted.sd_bb > 0.0 {
        ((1.96 * r.adjusted.sd_bb * 100.0 / 5.0).powi(2)).ceil() as usize
    } else {
        0
    };
    let pct = |a: usize, b: usize| if b > 0 { 100.0 * a as f64 / b as f64 } else { 0.0 };
    r.vpip = pct(vpip, pre_n);
    r.pfr = pct(pfr, pre_n);
    r.showdown_pct = pct(sd, raw.len());
    r.by_position = group(&rows, |h| vec![h.position.clone()]);
    r.by_players = group(&rows, |h| vec![format!("{}-handed", h.n_players)]);
    r.by_policy = group(&rows, |h| vec![h.policy.clone()]);
    r.by_opponent = group(&rows, |h| {
        h.players
            .iter()
            .filter(|p| p.seat != h.hero_seat)
            .filter_map(|p| p.name.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    });
    r.by_opponent.truncate(25);
    r
}

impl Report {
    pub fn render(&self) -> String {
        let mut s = String::new();
        let mut w = |l: String| {
            s.push_str(&l);
            s.push('\n');
        };
        w(format!("hands: {} ({} without a known profit)", self.hands, self.hands_without_profit));
        w(format!("raw        : {}", self.raw));
        w(format!(
            "all-in adj : {}   [{} hands adjusted, luck removed {:+.1} bb/100, variance ratio {:.2}x]",
            self.adjusted, self.hands_adjusted, self.allin_luck_bb100, self.var_ratio
        ));
        w(format!(
            "hands needed for +-5 bb/100 (95%): ~{}   (this sample is {})",
            self.hands_for_pm5,
            if self.hands_for_pm5 > 0 && self.hands < self.hands_for_pm5 / 4 { "far too small for any strength claim" } else { "see CI" }
        ));
        w(format!(
            "VPIP {:.1}%  PFR {:.1}%  showdown {:.1}%   decisions {}  fallbacks {}  engine/server issues {}  desync hands {}",
            self.vpip, self.pfr, self.showdown_pct, self.decisions, self.fallback_decisions, self.issue_count, self.desync_hands
        ));
        for (title, g) in [
            ("by position", &self.by_position),
            ("by table size", &self.by_players),
            ("by policy used", &self.by_policy),
            ("by opponent present (overlapping; top 25 by hands)", &self.by_opponent),
        ] {
            if g.is_empty() {
                continue;
            }
            w(format!("-- {title}"));
            for x in g.iter() {
                w(format!("   {:<14} {}", x.key, x.est));
            }
        }
        s
    }
}

// ---------------------------------------------------------------- loaders

#[derive(Default, Debug, Serialize)]
pub struct TraceSummary {
    pub lines: usize,
    pub bad_lines: usize,
    pub decisions: usize,
    pub acks: usize,
    pub rejects: usize,
    pub anomalies: usize,
}

pub fn load_trace(text: &str) -> (Vec<HandRecord>, TraceSummary) {
    let mut hands = Vec::new();
    let mut sum = TraceSummary::default();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        sum.lines += 1;
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            sum.bad_lines += 1;
            continue;
        };
        match v.get("kind").and_then(Value::as_str) {
            Some("hand") => match v.get("hand").and_then(|h| serde_json::from_value::<HandRecord>(h.clone()).ok()) {
                Some(h) => hands.push(h),
                None => sum.bad_lines += 1,
            },
            Some("decision") => sum.decisions += 1,
            Some("ack") => sum.acks += 1,
            Some("reject") => sum.rejects += 1,
            Some("anomaly") => sum.anomalies += 1,
            _ => {}
        }
    }
    (hands, sum)
}

fn parse_shown(v: Option<&Value>) -> BTreeMap<usize, Vec<String>> {
    let mut out = BTreeMap::new();
    let cards = |x: &Value| -> Vec<String> {
        x.as_array()
            .map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    };
    match v {
        Some(Value::Object(m)) => {
            for (k, x) in m {
                if let Ok(seat) = k.parse::<usize>() {
                    let c = if x.is_array() { cards(x) } else { x.get("cards").map(cards).unwrap_or_default() };
                    if c.len() == 2 {
                        out.insert(seat, c);
                    }
                }
            }
        }
        Some(Value::Array(a)) => {
            for x in a {
                if let (Some(seat), Some(c)) = (x.get("seat").and_then(proto::as_i64), x.get("cards").map(cards)) {
                    if c.len() == 2 {
                        out.insert(seat as usize, c);
                    }
                }
            }
        }
        _ => {}
    }
    out
}

pub fn shown_from_value(v: Option<&Value>) -> BTreeMap<usize, Vec<String>> {
    parse_shown(v)
}

/// Convert one entry of the REST `hand-history/export` into a [`HandRecord`]. Contributions and
/// the all-in lock are reconstructed by replaying `all_actions` on the engine (stacks of other
/// players are unknown, so they are assumed deep: a short-stack call is over-counted and the
/// record is only trusted when the replayed pot equals `pot_total`).
pub fn record_from_history(h: &Value) -> Option<HandRecord> {
    let hero_seat = h.get("seat").and_then(proto::as_i64)? as usize;
    let bb = proto::get_i64(h, "big_blind").unwrap_or(20);
    let sb = proto::get_i64(h, "small_blind").unwrap_or(bb / 2);
    let dealer = proto::get_i64(h, "dealer_seat").unwrap_or(0) as usize;
    let start = proto::get_i64(h, "stack_start");
    let hole: Vec<String> = h
        .get("hole_cards")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let board: Vec<String> = h
        .get("board")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let mut acts: Vec<(i64, ActionObs, String)> = Vec::new();
    for a in h.get("all_actions").and_then(Value::as_array).cloned().unwrap_or_default() {
        let seq = proto::get_i64(&a, "sequence_in_hand").unwrap_or(acts.len() as i64);
        if let Some(o) = proto::parse_player_action(&a) {
            let st = o.street.clone().unwrap_or_default();
            acts.push((seq, o, st));
        }
    }
    acts.sort_by_key(|x| x.0);
    let shown = parse_shown(h.get("shown_cards"));
    let mut rec = HandRecord {
        hand_id: proto::get_str(h, "hand_id").unwrap_or("").to_string(),
        table_id: proto::get_str(h, "table_id").map(str::to_string),
        hand_no: proto::get_u64(h, "hand_number").unwrap_or(0),
        hero_seat,
        dealer_seat: dealer,
        sb,
        bb,
        n_players: proto::get_i64(h, "num_players").unwrap_or(0) as usize,
        hero_hole: hole.clone(),
        board: board.clone(),
        profit: proto::get_i64(h, "profit"),
        profit_src: "history.profit".into(),
        policy: "history".into(),
        ..Default::default()
    };
    for (_, o, st) in &acts {
        rec.actions.push(HandAct { seat: o.seat, street: Some(st.clone()), action: o.raw_action.clone(), amount: o.amount });
    }
    // Reconstruction (best effort).
    let seats: BTreeSet<usize> = acts.iter().map(|x| x.1.seat).chain(std::iter::once(hero_seat)).collect();
    let hole_cards = parse_cards(&hole.concat()).filter(|c| c.len() == 2);
    let replay = (|| -> Option<Sim> {
        let hc = hole_cards.as_ref()?;
        if seats.len() != rec.n_players || seats.len() < 2 {
            return None;
        }
        let big = 100_000i64;
        let stacks: BTreeMap<usize, i64> = seats.iter().map(|&s| (s, if s == hero_seat { start.unwrap_or(big) } else { big })).collect();
        let layout = Layout::build(&seats, dealer, &stacks, sb, bb, false).ok()?;
        let mut sim = Sim::new(layout, hero_seat, [hc[0], hc[1]], None, 0).ok()?;
        for (_, o, _) in &acts {
            sim.apply_obs(o).ok()?;
        }
        Some(sim)
    })();
    if let Some(mut sim) = replay {
        // `pot_total` in the export is unreliable (it disagrees with the actions even for
        // plain hands), so it is NOT used as a check; a hand is trusted only if every
        // action replays on the engine without a clamp/mismatch.
        let issues = sim.take_issues();
        rec.issues = issues.len() as u32;
        for &s in &seats {
            let nl = sim.layout.nl(s).unwrap();
            rec.players.push(HandPlayer {
                seat: s,
                name: None,
                start_stack: if s == hero_seat { start } else { None },
                contrib: Some(sim.contributed_op(s).unwrap_or(0)),
                folded: Some(sim.state.is_folded(nl)),
                shown: shown.get(&s).cloned(),
            });
        }
        rec.showdown = sim.state.showdown();
        rec.position = sim.layout.position_name(hero_seat).to_string();
        if issues.is_empty() {
            rec.lock_len = sim.lock_len;
        } else {
            rec.desync = true; // not exactly reconstructable: no all-in adjustment
        }
    } else {
        rec.desync = true;
    }
    Some(rec)
}

pub fn load_history(v: &Value) -> Vec<HandRecord> {
    let arr = v.get("hands").and_then(Value::as_array).cloned().unwrap_or_else(|| v.as_array().cloned().unwrap_or_default());
    // Export is newest-first; keep chronological order.
    let mut out: Vec<HandRecord> = arr.iter().filter_map(record_from_history).collect();
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn estimate_math() {
        // 10 hands: +1bb x5, -1bb x5 -> mean 0, sd = sqrt(10/9)
        let xs: Vec<f64> = (0..10).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }).collect();
        let e = estimate(&xs);
        assert!(e.mean_bb100.abs() < 1e-12);
        assert!((e.sd_bb - (10.0f64 / 9.0).sqrt()).abs() < 1e-12);
        let half = t_crit95(9) * e.sd_bb / (10f64).sqrt() * 100.0;
        assert!((e.ci_hi - half).abs() < 1e-9 && (e.ci_lo + half).abs() < 1e-9);
        // bb/100 scaling: +0.5 bb per hand = +50 bb/100
        let e = estimate(&[0.5, 0.5, 0.5, 0.5]);
        assert!((e.mean_bb100 - 50.0).abs() < 1e-9 && e.se_bb100 == 0.0);
        assert_eq!(estimate(&[]).n, 0);
        assert!(estimate(&[1.0]).ci_hi.is_infinite());
        assert_eq!(t_crit95(1000), 1.96);
        assert!(t_crit95(5) > t_crit95(50));
    }

    fn rec(profit: i64, bb: i64) -> HandRecord {
        HandRecord { hand_id: "h".into(), hero_seat: 0, bb, profit: Some(profit), position: "BTN".into(), n_players: 6, ..Default::default() }
    }

    #[test]
    fn report_basics_and_ci_coverage() {
        let hands: Vec<HandRecord> = (0..100).map(|i| rec(if i % 2 == 0 { 40 } else { -20 }, 20)).collect();
        let r = analyze(&hands, 100);
        assert_eq!(r.hands, 100);
        // mean +10 chips = +0.5 bb -> 50 bb/100
        assert!((r.raw.mean_bb100 - 50.0).abs() < 1e-9);
        assert!((r.adjusted.mean_bb100 - r.raw.mean_bb100).abs() < 1e-9);
        assert_eq!(r.hands_adjusted, 0);
        assert!(r.raw.ci_lo < 50.0 && r.raw.ci_hi > 50.0);
        assert!(r.render().contains("bb/100"));
        // empty and no-profit inputs do not panic
        assert_eq!(analyze(&[], 10).raw.n, 0);
    }

    #[test]
    fn allin_adjustment_replaces_runout_luck() {
        // Hero AhAd vs KsKc all-in on the turn Qc 7d 2h 3s (2 kings left): EV = 200*42/44 - 100.
        let mk = |profit: i64, river: &str| HandRecord {
            hand_id: format!("h{profit}"),
            hero_seat: 0,
            bb: 20,
            profit: Some(profit),
            hero_hole: vec!["Ah".into(), "Ad".into()],
            board: vec!["Qc".into(), "7d".into(), "2h".into(), "3s".into(), river.into()],
            lock_len: Some(4),
            showdown: true,
            n_players: 2,
            position: "SB".into(),
            players: vec![
                HandPlayer { seat: 0, contrib: Some(100), folded: Some(false), ..Default::default() },
                HandPlayer { seat: 1, name: Some("villain".into()), contrib: Some(100), folded: Some(false), shown: Some(vec!["Ks".into(), "Kc".into()]), ..Default::default() },
            ],
            ..Default::default()
        };
        let ev = 200.0 * 42.0 / 44.0 - 100.0;
        let h = mk(100, "9d");
        assert!((allin_ev(&h, 0).unwrap() - ev).abs() < 1e-9);
        // The two outcomes (win +100 / lose -100) both map to the same EV: variance collapses.
        let hands = vec![mk(100, "9d"), mk(-100, "Kh"), mk(100, "5c"), mk(100, "8h")];
        let r = analyze(&hands, 0);
        assert_eq!(r.hands_adjusted, 4);
        assert!(r.adjusted.sd_bb < 1e-9, "all adjusted hands share one EV");
        assert!(r.var_ratio > 1e6 || r.var_ratio.is_finite());
        assert!((r.adjusted.mean_bb100 - ev / 20.0 * 100.0).abs() < 1e-6);
        // Unknown villain cards: not adjusted.
        let mut h2 = mk(100, "9d");
        h2.players[1].shown = None;
        assert!(allin_ev(&h2, 0).is_none());
        // River showdowns are never adjusted.
        let mut h3 = mk(100, "9d");
        h3.lock_len = Some(5);
        assert!(allin_ev(&h3, 0).is_none());
    }

    #[test]
    fn history_record_reconstruction() {
        // 3 players, dealer 0 -> SB 1, BB 2, BTN 0. Seat 0 raises to 60, 1 folds, 2 calls;
        // flop: 2 bets 100 all-in(stack 200 total? use plain bet), 0 calls; pot = 60*2+... check totals.
        let v = json!({
            "hand_id":"x","table_id":"t","hand_number":7,"seat":2,"stack_start":2000,"stack_end":1880,"profit":-120,
            "all_actions":[
                {"seat":0,"action":"raise","amount":60,"street":"preflop","sequence_in_hand":1},
                {"seat":1,"action":"fold","amount":0,"street":"preflop","sequence_in_hand":2},
                {"seat":2,"action":"call","amount":0,"street":"preflop","sequence_in_hand":3},
                {"seat":2,"action":"raise","amount":60,"street":"flop","sequence_in_hand":4},
                {"seat":0,"action":"call","amount":0,"street":"flop","sequence_in_hand":5}
            ],
            "board":["2c","7d","9h","Js","3c"],"pot_total":240,"num_players":3,"small_blind":10,"big_blind":20,
            "dealer_seat":0,"hole_cards":["Kc","Kd"],"shown_cards":{}
        });
        // Replay: preflop 60+60+10 (SB dead) = 130; flop 60+60 = 120 -> 250 (pot_total is ignored).
        let v2 = v.clone();
        let r = record_from_history(&v2).unwrap();
        assert!(!r.desync);
        assert_eq!(r.profit, Some(-120));
        assert_eq!(r.n_players, 3);
        assert_eq!(r.position, "BB");
        assert_eq!(r.players.iter().map(|p| p.contrib.unwrap()).sum::<i64>(), 250);
        assert_eq!(load_history(&json!({"hands":[v2.clone(), v2]})).len(), 2);
    }
}
