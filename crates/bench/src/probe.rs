//! Scripted "probe" opponents for diagnosing action-translation weaknesses. A probe never
//! folds and never plays for value: it just makes one kind of off-menu bet, so any large loss
//! of a blueprint against it is a translation/handling artefact, not poker skill.
//!
//! Spec (`probe:key=value,...`, all optional):
//! * `pre=X`  open-raise preflop to X big blinds when unopened (`pre=min` = min-raise,
//!   `pre=allin`); otherwise limp/call. `pp=P` probability of doing so (default 1).
//! * `bet=F`  postflop, when checked to, bet F x pot (clamped to the legal range); `bp=P`
//!   probability (default 1); `street=flop|turn|river|all` restricts it (default all).
//! * `raise=F` postflop, when facing exactly one bet, raise by F x (pot after calling);
//!   `rp=P` probability (default 1).
//! * `pre3=X` preflop facing exactly one raise: re-raise to X x the current bet (`p3=P`).
//! Everything else is check/call.

use crate::policy::{Dist, Player, PolicyFactory, View};
use cards::Card;
use nlhe::{Action, GameConfig, State, Street};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Probe {
    spec: String,
    /// Preflop open size in big blinds: `-1` = min-raise, `f64::INFINITY` = all-in, `0` = off.
    pub pre: f64,
    pub pp: f64,
    pub pre3: f64,
    pub p3: f64,
    pub bet: f64,
    pub bp: f64,
    /// Bitmask of streets (1 = flop, 2 = turn, 4 = river) where `bet` applies.
    pub bet_streets: u8,
    pub raise: f64,
    pub rp: f64,
}

pub fn parse(spec: &str) -> Result<Arc<dyn PolicyFactory>, String> {
    let body = spec.strip_prefix("probe:").ok_or("not a probe spec")?;
    let mut p = Probe {
        spec: spec.to_string(),
        pre: 0.0,
        pp: 1.0,
        pre3: 0.0,
        p3: 1.0,
        bet: 0.0,
        bp: 1.0,
        bet_streets: 7,
        raise: 0.0,
        rp: 1.0,
    };
    for kv in body.split(',').filter(|s| !s.is_empty()) {
        let (k, v) = kv.split_once('=').ok_or(format!("bad probe field {kv}"))?;
        let num = || v.parse::<f64>().map_err(|e| format!("{kv}: {e}"));
        match k {
            "pre" => {
                p.pre = match v {
                    "min" => -1.0,
                    "allin" => f64::INFINITY,
                    _ => num()?,
                }
            }
            "pp" => p.pp = num()?,
            "pre3" => {
                p.pre3 = match v {
                    "allin" => f64::INFINITY,
                    _ => num()?,
                }
            }
            "p3" => p.p3 = num()?,
            "bet" => p.bet = num()?,
            "bp" => p.bp = num()?,
            "street" => {
                p.bet_streets = match v {
                    "flop" => 1,
                    "turn" => 2,
                    "river" => 4,
                    "all" => 7,
                    _ => return Err(format!("unknown street {v}")),
                }
            }
            "raise" => p.raise = num()?,
            "rp" => p.rp = num()?,
            _ => return Err(format!("unknown probe field {k}")),
        }
    }
    Ok(Arc::new(p))
}

impl PolicyFactory for Probe {
    fn name(&self) -> String {
        self.spec.clone()
    }
    fn new_player(&self, _seat: usize, _cfg: &GameConfig, _seed: u64) -> Box<dyn Player> {
        Box::new(self.clone())
    }
}

fn mix(passive: Action, aggressive: Action, p: f64) -> Dist {
    if p >= 1.0 {
        return Dist::pure(aggressive);
    }
    let mut d = Dist::new();
    d.push(aggressive, p as f32);
    d.push(passive, (1.0 - p) as f32);
    d
}

impl Player for Probe {
    fn dist(&mut self, v: &View, _hole: [Card; 2]) -> Dist {
        let st: &State = v.state;
        let l = st.legal();
        let passive = if l.can_check { Action::Check } else { Action::Call };
        if !l.can_raise {
            return Dist::pure(passive);
        }
        let raises = st.raises_this_street();
        let clamp = |to: f64| Action::RaiseTo((to.round() as i64).clamp(l.min_raise_to as i64, l.max_raise_to as i64) as i32);
        let frac_to = |f: f64| {
            if f.is_infinite() {
                return Action::RaiseTo(l.max_raise_to);
            }
            clamp(st.cur_bet() as f64 + f * (st.pot() + l.call_amount) as f64)
        };
        let bb = v.cfg.big_blind as f64;
        match st.street() {
            Street::Preflop => {
                if raises == 0 && self.pre != 0.0 {
                    let a = if self.pre < 0.0 {
                        Action::RaiseTo(l.min_raise_to)
                    } else if self.pre.is_infinite() {
                        Action::RaiseTo(l.max_raise_to)
                    } else {
                        clamp(self.pre * bb)
                    };
                    return mix(passive, a, self.pp);
                }
                if raises == 1 && self.pre3 != 0.0 {
                    let a = if self.pre3.is_infinite() {
                        Action::RaiseTo(l.max_raise_to)
                    } else {
                        clamp(self.pre3 * st.cur_bet() as f64)
                    };
                    return mix(passive, a, self.p3);
                }
                Dist::pure(passive)
            }
            s => {
                let bit = 1u8 << (s.idx() - 1);
                if raises == 0 && self.bet > 0.0 && self.bet_streets & bit != 0 {
                    return mix(passive, frac_to(self.bet), self.bp);
                }
                if raises == 1 && self.raise > 0.0 {
                    return mix(passive, frac_to(self.raise), self.rp);
                }
                Dist::pure(passive)
            }
        }
    }
    fn fork(&self) -> Option<Box<dyn Player>> {
        Some(Box::new(self.clone()))
    }
}
