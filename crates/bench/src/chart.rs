//! `chart6`: a deterministic, rule-based 6-max bot in the style of a "GTO-chart + pot-odds"
//! player (what a disciplined human reg or a commercial rule-based bot does), implemented
//! natively so it can be a fast, reproducible opponent.
//!
//! * Preflop: position-dependent open-raise charts (hand lists in the shape of commonly
//!   published 100 bb 6-max RFI charts: UTG ~16 %, HJ ~21 %, CO ~27 %, BTN ~44 %, SB ~42 %
//!   raise-or-fold), then 3-bet / call / 4-bet / 5-bet-jam bands that depend on the
//!   opener's and hero's position. Response bands are percentile cut-offs on a hand
//!   ranking (equity vs a random hand, computed once at start-up). These are *approximations*
//!   of published charts written from memory, not a copy of any solver output.
//! * Postflop: Monte-Carlo equity against a range proxy (top fraction of villain's hands by
//!   expected future strength, narrower when facing bigger bets or raises), then pot-odds
//!   calls, value bets/raises, c-bets and a few hash-driven bluffs.
//!
//! Everything is a pure function of (public history, hole cards): the "randomness" (bluffs,
//! MC sampling) is seeded from a hash of the situation, so `dist` is a deterministic point
//! mass and AIVAT can treat the bot as a known strategy.

use crate::mix;
use crate::policy::{Dist, Player, PolicyFactory, View};
use cards::{bit, cards_in, evaluate, mask_of, Card, Hand, Rng, FULL_DECK};
use nlhe::{Action, GameConfig, Street};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pos {
    Sb,
    Bb,
    Utg,
    Hj,
    Co,
    Btn,
}

fn pos_of(seat: usize, n: usize) -> Pos {
    if n == 2 {
        return if seat == 0 { Pos::Btn } else { Pos::Bb };
    }
    match seat {
        0 => Pos::Sb,
        1 => Pos::Bb,
        s if s == n - 1 => Pos::Btn,
        s if s == n - 2 => Pos::Co,
        s if s == n - 3 => Pos::Hj,
        _ => Pos::Utg,
    }
}

/// 13x13 class membership parsed from tokens like `22+`, `A2s+`, `ATo+`, `T9s`, `KQo`.
fn parse_range(s: &str) -> [bool; 169] {
    let mut out = [false; 169];
    let rk = |c: char| "23456789TJQKA".find(c).unwrap();
    let class = |hi: usize, lo: usize, suited: bool| {
        if hi == lo {
            hi
        } else {
            let tri = hi * (hi - 1) / 2 + lo;
            if suited {
                13 + tri
            } else {
                91 + tri
            }
        }
    };
    for tok in s.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        let c: Vec<char> = tok.chars().collect();
        let (a, b) = (rk(c[0]), rk(c[1]));
        let plus = tok.ends_with('+');
        let suited = c.get(2) == Some(&'s');
        if a == b {
            let top = if plus { 12 } else { a };
            for r in a..=top {
                out[class(r, r, false)] = true;
            }
        } else {
            let (hi, lo) = (a.max(b), a.min(b));
            let top = if plus { hi - 1 } else { lo };
            for l in lo..=top {
                out[class(hi, l, suited)] = true;
            }
        }
    }
    out
}

struct Tables {
    /// Percentile (0 = best) of each class: fraction of combos strictly better + half of own.
    pct: [f32; 169],
    rfi: [[bool; 169]; 6],
    /// Fraction of combos in each RFI chart (for the scaled styles).
    rfi_width: [f32; 6],
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        // Equity vs a random hand for each class (deterministic MC).
        let mut eq = [0f64; 169];
        let mut combos = [0f64; 169];
        let mut rng = Rng::new(0xC0FFEE);
        for hi in 0..13u8 {
            for lo in 0..=hi {
                for suited in [false, true] {
                    if hi == lo && suited {
                        continue;
                    }
                    let (c1, c2) = (cards::card(hi, 0), cards::card(lo, if suited { 0 } else { 1 }));
                    let cls = cards::preflop_class(c1, c2);
                    combos[cls] = if hi == lo { 6.0 } else if suited { 4.0 } else { 12.0 };
                    let (mut w, n) = (0.0, 3000);
                    for _ in 0..n {
                        let mut dead: Hand = bit(c1) | bit(c2);
                        let o1 = rng.draw(dead);
                        dead |= bit(o1);
                        let o2 = rng.draw(dead);
                        dead |= bit(o2);
                        let mut board: Hand = 0;
                        for _ in 0..5 {
                            let c = rng.draw(dead | board);
                            board |= bit(c);
                        }
                        let (a, b) = (evaluate(bit(c1) | bit(c2) | board), evaluate(bit(o1) | bit(o2) | board));
                        w += if a > b { 1.0 } else if a == b { 0.5 } else { 0.0 };
                    }
                    eq[cls] = w / n as f64;
                }
            }
        }
        let total: f64 = combos.iter().sum();
        let mut pct = [0f32; 169];
        for i in 0..169 {
            if combos[i] == 0.0 {
                pct[i] = 1.0;
                continue;
            }
            let (mut better, mut same) = (0.0, 0.0);
            for j in 0..169 {
                if eq[j] > eq[i] {
                    better += combos[j];
                } else if eq[j] == eq[i] {
                    same += combos[j];
                }
            }
            pct[i] = ((better + same / 2.0) / total) as f32;
        }
        let rfi = [
            // SB (raise-or-fold)
            parse_range("22+,A2s+,K2s+,Q4s+,J6s+,T6s+,96s+,86s+,75s+,65s,54s,A2o+,K8o+,Q9o+,J9o+,T9o"),
            // BB (unused as an opener; iso-raise range vs limpers)
            parse_range("77+,ATs+,KJs+,AJo+,KQo"),
            // UTG
            parse_range("22+,A4s+,K9s+,Q9s+,J9s+,T9s,98s,87s,76s,ATo+,KQo"),
            // HJ
            parse_range("22+,A2s+,K8s+,Q9s+,J9s+,T8s+,97s+,87s,76s,65s,ATo+,KJo+,QJo"),
            // CO
            parse_range("22+,A2s+,K5s+,Q8s+,J8s+,T8s+,97s+,86s+,75s+,65s,54s,A9o+,KTo+,QTo+,JTo"),
            // BTN
            parse_range("22+,A2s+,K2s+,Q4s+,J6s+,T6s+,96s+,85s+,75s+,64s+,54s,43s,A2o+,K8o+,Q9o+,J9o+,T9o,98o"),
        ];
        let rfi_width = std::array::from_fn(|i| {
            (0..169).filter(|&c| rfi[i][c]).map(|c| combos[c]).sum::<f64>() as f32 / 1326.0
        });
        Tables { pct, rfi, rfi_width }
    })
}

fn pos_idx(p: Pos) -> usize {
    match p {
        Pos::Sb => 0,
        Pos::Bb => 1,
        Pos::Utg => 2,
        Pos::Hj => 3,
        Pos::Co => 4,
        Pos::Btn => 5,
    }
}

/// (3-bet top-%, call top-% above the 3-bet band) for hero position vs opener position.
fn response_band(hero: Pos, opener: Pos) -> (f32, f32) {
    let o = match opener {
        Pos::Utg => 0,
        Pos::Hj => 1,
        Pos::Co => 2,
        Pos::Btn => 3,
        Pos::Sb => 4,
        Pos::Bb => 4,
    };
    let t: [(f32, f32); 5] = match hero {
        Pos::Bb => [(0.05, 0.17), (0.06, 0.22), (0.08, 0.28), (0.12, 0.40), (0.13, 0.46)],
        Pos::Sb => [(0.05, 0.0), (0.06, 0.0), (0.08, 0.02), (0.12, 0.06), (0.12, 0.06)],
        Pos::Btn => [(0.04, 0.07), (0.05, 0.09), (0.08, 0.13), (0.08, 0.13), (0.08, 0.13)],
        Pos::Co => [(0.04, 0.06), (0.05, 0.08), (0.05, 0.08), (0.05, 0.08), (0.05, 0.08)],
        Pos::Hj => [(0.04, 0.05), (0.04, 0.05), (0.04, 0.05), (0.04, 0.05), (0.04, 0.05)],
        Pos::Utg => [(0.04, 0.04), (0.04, 0.04), (0.04, 0.04), (0.04, 0.04), (0.04, 0.04)],
    };
    t[o]
}

/// The chart bot; `style` scales every preflop width and band and the postflop aggression:
/// 1.0 = `chart6` (the published-chart shape), 1.45 = `chart6-loose` (loose-aggressive),
/// 0.7 = `chart6-tight` (nit).
pub struct Chart6 {
    pub style: f32,
}

impl Chart6 {
    pub const STD: Chart6 = Chart6 { style: 1.0 };
    pub fn by_name(name: &str) -> Option<Chart6> {
        match name {
            "chart6" => Some(Chart6 { style: 1.0 }),
            "chart6-loose" => Some(Chart6 { style: 1.45 }),
            "chart6-tight" => Some(Chart6 { style: 0.7 }),
            _ => None,
        }
    }
}

impl PolicyFactory for Chart6 {
    fn name(&self) -> String {
        match self.style {
            s if s > 1.1 => "chart6-loose",
            s if s < 0.9 => "chart6-tight",
            _ => "chart6",
        }
        .into()
    }
    fn new_player(&self, _seat: usize, _cfg: &GameConfig, _seed: u64) -> Box<dyn Player> {
        Box::new(ChartPlayer { style: self.style })
    }
}

#[derive(Clone)]
struct ChartPlayer {
    style: f32,
}

fn situation_hash(v: &View, hole: [Card; 2]) -> u64 {
    let mut h = mix(0x51ED, (bit(hole[0]) | bit(hole[1])) ^ mask_of(v.board).rotate_left(7));
    h = mix(h, v.history.len() as u64 * 131 + v.seat as u64);
    for e in v.history.iter().rev().take(6) {
        h = mix(h, e.seat as u64 * 7919 + e.street.idx() as u64 * 31 + action_code(e.action));
    }
    h
}

fn action_code(a: Action) -> u64 {
    match a {
        Action::Fold => 1,
        Action::Check => 2,
        Action::Call => 3,
        Action::RaiseTo(x) => 4 + x as u64 * 8,
    }
}

fn unit(h: u64) -> f64 {
    (h >> 11) as f64 / (1u64 << 53) as f64
}

impl Player for ChartPlayer {
    fn fork(&self) -> Option<Box<dyn Player>> {
        Some(Box::new(self.clone()))
    }
    fn dist(&mut self, v: &View, hole: [Card; 2]) -> Dist {
        let a = if v.state.street() == Street::Preflop {
            preflop(v, hole, self.style)
        } else {
            postflop(v, hole, self.style)
        };
        Dist::pure(legalize(v, a))
    }
}

/// Make any intent legal: raise sizes clamped, impossible raises become calls/checks, folds
/// of a free check become checks.
fn legalize(v: &View, a: Action) -> Action {
    let l = v.state.legal();
    match a {
        Action::RaiseTo(t) => {
            if l.can_raise {
                let t = t.clamp(l.min_raise_to, l.max_raise_to);
                // Do not leave a sliver behind: jam when the raise commits most of the stack.
                let st = v.state;
                let rem = l.max_raise_to - t;
                if rem * 4 < (t - st.street_bet(v.seat)).max(1) {
                    Action::RaiseTo(l.max_raise_to)
                } else {
                    Action::RaiseTo(t)
                }
            } else if l.can_check {
                Action::Check
            } else {
                Action::Call
            }
        }
        Action::Fold if l.can_check => Action::Check,
        Action::Check if !l.can_check => Action::Call,
        Action::Call if l.can_check => Action::Check,
        x => x,
    }
}

fn preflop(v: &View, hole: [Card; 2], style: f32) -> Action {
    let t = tables();
    let st = v.state;
    let n = st.num_players();
    let pos = pos_of(v.seat, n);
    let bb = v.cfg.big_blind;
    let cls = cards::preflop_class(hole[0], hole[1]);
    let pct = t.pct[cls];
    let raises = st.raises_this_street();
    let me_bet = st.street_bet(v.seat);
    let cur = st.cur_bet();
    let l = st.legal();
    let preflop_events: Vec<_> = v.history.iter().filter(|e| e.street == Street::Preflop).collect();
    let limpers = preflop_events.iter().filter(|e| e.action == Action::Call).count() as i32;
    let in_position = matches!(pos, Pos::Btn | Pos::Co);

    if raises == 0 {
        let open_sz = if matches!(pos, Pos::Sb) { bb * 3 } else { bb * 5 / 2 };
        if pos == Pos::Bb && l.can_check {
            // Option: raise only the very top over limpers.
            if limpers > 0 && pct < 0.07 {
                return Action::RaiseTo(bb * 35 / 10 + limpers * bb);
            }
            return Action::Check;
        }
        let in_range = if style == 1.0 {
            t.rfi[pos_idx(pos)][cls]
        } else {
            pct < t.rfi_width[pos_idx(pos)] * style
        };
        if in_range {
            return Action::RaiseTo(open_sz + limpers * bb);
        }
        return Action::Fold;
    }

    let opener = preflop_events
        .iter()
        .find(|e| matches!(e.action, Action::RaiseTo(_)))
        .map(|e| pos_of(e.seat as usize, n))
        .unwrap_or(Pos::Utg);
    let callers = preflop_events.iter().filter(|e| e.action == Action::Call).count() as i32;
    let i_raised = me_bet > bb;
    let pot_after_call = st.pot() + l.call_amount;
    let shove_zone = |to: i32| to * 10 >= (me_bet + st.stack(v.seat)) * 4;
    let big = |to: i32| if shove_zone(to) { l.max_raise_to } else { to };

    match raises {
        1 => {
            let (b3, bc) = response_band(pos, opener);
            let (b3, bc) = (b3 * style, bc * style);
            // Wider callers behind us -> squeeze bigger; multiway -> tighten the calling band.
            if pct < b3 {
                let to = if in_position { cur * 3 } else { cur * 37 / 10 } + callers * cur;
                Action::RaiseTo(big(to))
            } else if pct < b3 + bc * if callers > 0 { 0.7 } else { 1.0 } {
                Action::Call
            } else {
                Action::Fold
            }
        }
        2 => {
            if i_raised {
                let (b4, bc) = (0.025 * style, (if in_position { 0.09 } else { 0.07 }) * style);
                if pct < b4 {
                    Action::RaiseTo(big(cur * 23 / 10))
                } else if pct < b4 + bc && pot_after_call > 0 {
                    Action::Call
                } else {
                    Action::Fold
                }
            } else if pct < 0.012 {
                Action::RaiseTo(big(cur * 23 / 10))
            } else if pct < 0.03 {
                Action::Call
            } else {
                Action::Fold
            }
        }
        _ => {
            if pct < 0.010 {
                Action::RaiseTo(l.max_raise_to)
            } else if pct < 0.024 {
                Action::Call
            } else {
                Action::Fold
            }
        }
    }
}

/// Expected-future-strength score of a villain combo on the current board.
fn future_score(c: [Card; 2], board: Hand, bl: usize, fut: &[Card]) -> f64 {
    let base = bit(c[0]) | bit(c[1]) | board;
    if bl == 5 {
        return evaluate(base) as f64;
    }
    let mut s = 0.0;
    let mut k = 0.0;
    for &f in fut {
        if bit(f) & base == 0 {
            s += evaluate(base | bit(f)) as f64;
            k += 1.0;
        }
    }
    if k == 0.0 {
        evaluate(base) as f64
    } else {
        s / k
    }
}

fn postflop(v: &View, hole: [Card; 2], style: f32) -> Action {
    let st = v.state;
    let l = st.legal();
    let bl = v.board.len();
    let board = mask_of(v.board);
    let hm = bit(hole[0]) | bit(hole[1]);
    let h = situation_hash(v, hole);
    let mut rng = Rng::new(h);
    let pot = st.pot();
    let bb = v.cfg.big_blind;
    let opps = (st.live_count() as i32 - 1).max(1);
    let facing = l.call_amount > 0;
    let raises = st.raises_this_street();
    let eff = (0..st.num_players())
        .filter(|&s| s != v.seat && !st.is_folded(s))
        .map(|s| st.stack(s) + st.street_bet(s))
        .max()
        .unwrap_or(0)
        .min(st.stack(v.seat) + st.street_bet(v.seat));

    // Villain range proxy: top `keep` fraction by expected future strength.
    let frac_bet = if facing {
        l.call_amount as f64 / (pot - l.call_amount).max(bb) as f64
    } else {
        0.0
    };
    let keep = if !facing {
        0.85
    } else if raises >= 2 {
        0.20
    } else {
        (0.78 - 0.36 * frac_bet.min(1.3)).clamp(0.28, 0.75)
    };
    let fut: Vec<Card> = {
        let mut r = Rng::new(h ^ 0xABCD);
        let mut dead = board | hm;
        (0..6)
            .map(|_| {
                let c = r.draw(dead);
                dead |= bit(c);
                c
            })
            .collect()
    };
    let mut combos: Vec<([Card; 2], f64)> = Vec::with_capacity(1100);
    let free = FULL_DECK & !(board | hm);
    let cs: Vec<Card> = cards_in(free).collect();
    for i in 0..cs.len() {
        for j in i + 1..cs.len() {
            combos.push(([cs[i], cs[j]], future_score([cs[i], cs[j]], board, bl, &fut)));
        }
    }
    combos.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    let kept = ((combos.len() as f64 * keep) as usize).max(8);
    let range = &combos[..kept];

    let samples = 220;
    let mut wins = 0.0;
    for _ in 0..samples {
        let o = range[rng.below(range.len() as u32) as usize].0;
        let mut b = board;
        let mut dead = board | hm | bit(o[0]) | bit(o[1]);
        for _ in bl..5 {
            let c = rng.draw(dead);
            dead |= bit(c);
            b |= bit(c);
        }
        let (x, y) = (evaluate(hm | b), evaluate(bit(o[0]) | bit(o[1]) | b));
        wins += if x > y { 1.0 } else if x == y { 0.5 } else { 0.0 };
    }
    let eq = wins / samples as f64;
    let multi = 0.05 * (opps - 1) as f64;
    let u = unit(mix(h, 99));

    // Did we take the last preflop raise? (c-bet candidate)
    let last_pf_raiser = v
        .history
        .iter()
        .filter(|e| e.street == Street::Preflop && matches!(e.action, Action::RaiseTo(_)))
        .last()
        .map(|e| e.seat as usize);
    let aggressor = last_pf_raiser == Some(v.seat);

    let size = |frac: f64| Action::RaiseTo(st.cur_bet() + (frac * (pot + l.call_amount) as f64).round() as i32);
    if !facing {
        let value_thr = 0.64 + multi - 0.08 * (style as f64 - 1.0);
        if eq >= value_thr {
            let f = if eq > 0.88 { if bl == 5 { 1.0 } else { 0.75 } } else { 0.5 };
            return size(f);
        }
        if aggressor && bl == 3 && opps <= 2 && eq >= 0.40 {
            return size(0.33);
        }
        if (0.26..0.42).contains(&eq) && bl < 5 && u < (if aggressor { 0.30 } else { 0.12 }) * style as f64 {
            return size(0.5);
        }
        return Action::Check;
    }
    let call = l.call_amount as f64;
    let po = call / (pot as f64 + call);
    let spr_low = eff as f64 <= 1.2 * (pot as f64);
    if eq >= 0.83 + multi && raises <= 1 {
        return if spr_low || eq > 0.94 {
            Action::RaiseTo(l.max_raise_to)
        } else {
            Action::RaiseTo(st.cur_bet() * 3)
        };
    }
    if (0.34..0.46).contains(&eq) && bl < 5 && raises <= 1 && u < 0.07 * style as f64 && !spr_low {
        return Action::RaiseTo(st.cur_bet() * 3);
    }
    if eq >= po + (0.03 + multi) / style as f64 {
        Action::Call
    } else {
        Action::Fold
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matchrun::play_hand;
    use cards::Rng;
    use nlhe::Deal;

    #[test]
    fn charts_have_expected_widths() {
        let t = tables();
        let combos = |r: &[bool; 169]| {
            (0..169)
                .filter(|&i| r[i])
                .map(|i| if i < 13 { 6 } else if i < 91 { 4 } else { 12 })
                .sum::<u32>() as f64
                / 1326.0
        };
        let w: Vec<f64> = (0..6).map(|i| combos(&t.rfi[i])).collect();
        assert!((0.14..0.20).contains(&w[2]), "UTG {:?}", w);
        assert!((0.18..0.25).contains(&w[3]), "HJ {:?}", w);
        assert!((0.24..0.31).contains(&w[4]), "CO {:?}", w);
        assert!((0.40..0.48).contains(&w[5]), "BTN {:?}", w);
        // AA is the best hand, 72o among the worst.
        assert!(t.pct[12] < 0.01 && t.pct[cards::preflop_class(cards::card(5, 0), cards::card(0, 1))] > 0.9);
    }

    #[test]
    fn plays_legal_hands_and_is_deterministic() {
        let cfg = GameConfig::new(6);
        let f = Chart6::STD;
        let mut rng = Rng::new(5);
        let mut tot = 0i64;
        for k in 0..60 {
            let deal = Deal::random(&mut rng);
            let mk = || (0..6).map(|s| f.new_player(s, &cfg, k)).collect::<Vec<_>>();
            let mut a = mk();
            let o1 = play_hand(&cfg, &deal, &mut a, &mut Rng::new(k), None);
            let mut b = mk();
            let o2 = play_hand(&cfg, &deal, &mut b, &mut Rng::new(k + 1000), None);
            assert_eq!(o1.payoffs, o2.payoffs, "chart6 must not depend on the random stream");
            assert_eq!(o1.payoffs.iter().sum::<i32>(), 0);
            tot += o1.actions as i64;
        }
        assert!(tot > 60 * 3);
    }
}
