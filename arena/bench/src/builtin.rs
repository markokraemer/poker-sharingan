//! Gauntlet opponents: simple heuristics on preflop hand percentile and postflop
//! made-hand percentile (current strength vs a random hand, no draws). Not tuned.

use crate::policy::{Dist, Player, PolicyFactory, View};
use cards::{all_holes, bit, evaluate, mask_of, Card, Hand};
use nlhe::{Action, GameConfig, Street};
use std::sync::{Arc, OnceLock};

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Random,
    Station,
    Heur(Style),
}

#[derive(Clone, Copy, Debug)]
pub struct Style {
    /// Preflop percentile (0..1, higher = better hand) needed to open-raise.
    pub open: f32,
    /// ... to re-raise a raise.
    pub reraise: f32,
    /// ... to call a raise.
    pub call_pre: f32,
    /// Postflop percentile to bet / raise for value.
    pub bet: f32,
    /// Postflop percentile to continue vs a bet.
    pub call_post: f32,
    /// Probability of raising when the hand is strong enough.
    pub aggr: f32,
    /// Probability of bluff-raising otherwise.
    pub bluff: f32,
    /// Postflop bet size, pot fraction.
    pub size: f32,
}

pub const NIT: Style = Style {
    open: 0.88,
    reraise: 0.97,
    call_pre: 0.90,
    bet: 0.85,
    call_post: 0.70,
    aggr: 0.8,
    bluff: 0.01,
    size: 0.66,
};
pub const TAG: Style = Style {
    open: 0.75,
    reraise: 0.93,
    call_pre: 0.80,
    bet: 0.70,
    call_post: 0.50,
    aggr: 0.85,
    bluff: 0.08,
    size: 0.66,
};
pub const LAG: Style = Style {
    open: 0.55,
    reraise: 0.85,
    call_pre: 0.60,
    bet: 0.55,
    call_post: 0.35,
    aggr: 0.85,
    bluff: 0.20,
    size: 0.75,
};
pub const MANIAC: Style = Style {
    open: 0.30,
    reraise: 0.60,
    call_pre: 0.35,
    bet: 0.35,
    call_post: 0.25,
    aggr: 0.95,
    bluff: 0.45,
    size: 1.0,
};

pub struct Builtin {
    pub name: &'static str,
    pub kind: Kind,
}

pub fn all_builtins() -> Vec<Arc<dyn PolicyFactory>> {
    ["random", "station", "nit", "tag", "lag", "maniac"]
        .iter()
        .map(|n| builtin(n).unwrap())
        .collect()
}

pub fn builtin(name: &str) -> Option<Arc<dyn PolicyFactory>> {
    let (name, kind) = match name {
        "random" => ("random", Kind::Random),
        "station" => ("station", Kind::Station),
        "nit" => ("nit", Kind::Heur(NIT)),
        "tag" => ("tag", Kind::Heur(TAG)),
        "lag" => ("lag", Kind::Heur(LAG)),
        "maniac" => ("maniac", Kind::Heur(MANIAC)),
        _ => return None,
    };
    Some(Arc::new(Builtin { name, kind }))
}

impl PolicyFactory for Builtin {
    fn name(&self) -> String {
        self.name.to_string()
    }
    fn new_player(&self, _seat: usize, _cfg: &GameConfig, _seed: u64) -> Box<dyn Player> {
        Box::new(HeurPlayer {
            kind: self.kind,
            cache_board: u64::MAX,
            sorted: Vec::new(),
        })
    }
}

#[derive(Clone)]
struct HeurPlayer {
    kind: Kind,
    cache_board: Hand,
    sorted: Vec<u32>,
}

/// Chen formula score.
fn chen(a: Card, b: Card) -> f32 {
    let (hi, lo) = (
        cards::rank_of(a).max(cards::rank_of(b)),
        cards::rank_of(a).min(cards::rank_of(b)),
    );
    let pts = |r: u8| match r {
        12 => 10.0,
        11 => 8.0,
        10 => 7.0,
        9 => 6.0,
        r => (r as f32 + 2.0) / 2.0,
    };
    let mut s = pts(hi);
    if hi == lo {
        return (s * 2.0).max(5.0).ceil();
    }
    if cards::suit_of(a) == cards::suit_of(b) {
        s += 2.0;
    }
    let gap = hi - lo - 1;
    s -= [0.0, 1.0, 2.0, 4.0][gap.min(3) as usize] + if gap >= 4 { 1.0 } else { 0.0 };
    if gap <= 1 && hi < 10 {
        s += 1.0;
    }
    s.ceil()
}

/// Preflop percentile of every hole pair (by Chen score), indexed by `hole_index`.
fn preflop_pct(hole: [Card; 2]) -> f32 {
    static T: OnceLock<Vec<f32>> = OnceLock::new();
    let t = T.get_or_init(|| {
        let hs = all_holes();
        let sc: Vec<f32> = hs.iter().map(|h| chen(h[0], h[1])).collect();
        sc.iter()
            .map(|&x| {
                let below = sc.iter().filter(|&&y| y < x).count() as f32;
                let eq = sc.iter().filter(|&&y| y == x).count() as f32;
                (below + 0.5 * eq) / sc.len() as f32
            })
            .collect()
    });
    t[cards::hole_index(hole[0], hole[1])]
}

impl HeurPlayer {
    /// Percentile of the made hand among all hole pairs on this board.
    fn post_pct(&mut self, hole: [Card; 2], board: &[Card]) -> f32 {
        let bm = mask_of(board);
        if self.cache_board != bm {
            self.cache_board = bm;
            self.sorted = all_holes()
                .iter()
                .filter(|h| bit(h[0]) & bm == 0 && bit(h[1]) & bm == 0)
                .map(|h| evaluate(bit(h[0]) | bit(h[1]) | bm))
                .collect();
            self.sorted.sort_unstable();
        }
        let r = evaluate(bit(hole[0]) | bit(hole[1]) | bm);
        let lo = self.sorted.partition_point(|&x| x < r);
        let hi = self.sorted.partition_point(|&x| x <= r);
        (lo as f32 + 0.5 * (hi - lo) as f32) / self.sorted.len() as f32
    }
}

fn soft(s: f32, th: f32) -> f32 {
    ((s - th) / 0.05 + 0.5).clamp(0.0, 1.0)
}

fn raise_target(v: &View, frac: f32, preflop_triple: bool) -> Option<Action> {
    let l = v.state.legal();
    if !l.can_raise {
        return None;
    }
    let to = if preflop_triple {
        v.state.cur_bet() * 3
    } else {
        v.state.cur_bet() + (frac * (v.state.pot() + l.call_amount) as f32).round() as i32
    };
    Some(Action::RaiseTo(to.clamp(l.min_raise_to, l.max_raise_to)))
}

impl Player for HeurPlayer {
    fn fork(&self) -> Option<Box<dyn Player>> {
        Some(Box::new(self.clone()))
    }

    fn dist(&mut self, v: &View, hole: [Card; 2]) -> Dist {
        let l = v.state.legal();
        let passive = if l.can_check {
            Action::Check
        } else {
            Action::Call
        };
        let mut d = Dist::new();
        match self.kind {
            Kind::Station => d.push(passive, 1.0),
            Kind::Random => {
                let r = raise_target(v, 1.0, false);
                let (pf, pr) = (
                    if l.can_fold { 0.15 } else { 0.0 },
                    if r.is_some() { 0.25 } else { 0.0 },
                );
                d.push(passive, 1.0 - pf - pr);
                if pf > 0.0 {
                    d.push(Action::Fold, pf);
                }
                if let Some(r) = r {
                    d.push(r, pr);
                }
            }
            Kind::Heur(st) => {
                let pre = v.state.street() == Street::Preflop;
                let s = if pre {
                    preflop_pct(hole)
                } else {
                    self.post_pct(hole, v.board)
                };
                let raised = v.state.raises_this_street() > 0;
                let (raise_th, call_th) = if pre {
                    (if raised { st.reraise } else { st.open }, st.call_pre)
                } else {
                    (st.bet, st.call_post)
                };
                let odds = l.call_amount as f32 / (v.state.pot() + l.call_amount).max(1) as f32;
                let call_th = if l.can_fold {
                    call_th.min(1.0) + 0.35 * odds * (1.0 - call_th)
                } else {
                    0.0
                };
                let strong = soft(s, raise_th);
                let mut pr = st.aggr * strong + st.bluff * (1.0 - strong);
                let rt = raise_target(v, st.size, pre);
                if rt.is_none() {
                    pr = 0.0;
                }
                if let Some(r) = rt {
                    d.push(r, pr);
                }
                let cont = if l.can_fold { soft(s, call_th) } else { 1.0 };
                d.push(passive, (1.0 - pr) * cont);
                if l.can_fold && cont < 1.0 {
                    d.push(Action::Fold, (1.0 - pr) * (1.0 - cont));
                }
            }
        }
        d
    }
}
