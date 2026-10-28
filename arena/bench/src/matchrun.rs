//! Seeded, parallel duplicate matches.
//!
//! For every sampled deal the same cards are replayed under each seating (a seat -> entrant
//! assignment). `Seatings::rotations(base)` cyclically shifts a base assignment so each
//! entrant plays every seat the base puts it in, e.g. `hero_vs_field(6)` puts the hero in
//! each of the 6 seats once against 5 copies of the field policy.
//!
//! Randomness: all streams are seeded per deal (`mix(seed, deal_index)`), so results do not
//! depend on rayon scheduling. With `crn` the k-th decision of a hand uses the k-th uniform of
//! a per-deal stream, identical across seatings (common random numbers).
//!
//! Statistics: an entrant's payoff in one seating is the mean over the seats it occupies.
//! The independent sample is the deal (mean over seatings). Everything is in mbb/game.

use crate::aivat::{EquityEstimator, Mode, Scratch, Tracker, ValueEstimator};
use crate::mix;
use crate::policy::{Event, PolicyFactory, View};
use crate::stats::{Acc, Est};
use cards::Rng;
use nlhe::{Deal, GameConfig, State, Step, MAX_PLAYERS};
use rayon::prelude::*;
use std::sync::Arc;

pub struct Seatings(pub Vec<Vec<usize>>);

impl Seatings {
    /// All cyclic rotations of `base` (seat -> entrant).
    pub fn rotations(base: &[usize]) -> Seatings {
        let n = base.len();
        Seatings(
            (0..n)
                .map(|r| (0..n).map(|s| base[(s + r) % n]).collect())
                .collect(),
        )
    }
    /// Entrant 0 (hero) in seat 0, entrant 1 (field) everywhere else, rotated.
    pub fn hero_vs_field(players: usize) -> Seatings {
        let mut base = vec![1; players];
        base[0] = 0;
        Self::rotations(&base)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AivatMode {
    Off,
    Full,
    CardsOnly,
}

pub struct MatchSpec {
    pub cfg: GameConfig,
    pub entrants: Vec<Arc<dyn PolicyFactory>>,
    pub seatings: Seatings,
    pub deals: u64,
    pub seed: u64,
    pub crn: bool,
    pub aivat: AivatMode,
    pub estimator: Arc<dyn ValueEstimator>,
}

impl MatchSpec {
    pub fn new(
        cfg: GameConfig,
        entrants: Vec<Arc<dyn PolicyFactory>>,
        seatings: Seatings,
        deals: u64,
        seed: u64,
    ) -> Self {
        MatchSpec {
            cfg,
            entrants,
            seatings,
            deals,
            seed,
            crn: true,
            aivat: AivatMode::Off,
            estimator: Arc::new(EquityEstimator::default()),
        }
    }
}

pub struct HandOutcome {
    /// Integer chip payoffs per seat.
    pub payoffs: [i32; MAX_PLAYERS],
    /// AIVAT-corrected payoffs (fractional-chop `u` + corrections), if tracked.
    pub aivat: Option<[f64; MAX_PLAYERS]>,
    pub actions: usize,
}

/// Play one hand with fresh players; `crn` supplies one uniform per decision.
pub fn play_hand(
    cfg: &GameConfig,
    deal: &Deal,
    players: &mut [Box<dyn crate::policy::Player>],
    crn: &mut Rng,
    mut tracker: Option<Tracker>,
) -> HandOutcome {
    let mut st = State::new(cfg);
    let mut len = 0usize;
    let mut hist: Vec<Event> = Vec::with_capacity(32);
    if let Some(t) = tracker.as_mut() {
        t.start(&st);
    }
    let mut actions = 0;
    while !st.is_terminal() {
        let seat = st.to_act().unwrap();
        let u = crn.f64();
        let view = View {
            cfg,
            state: &st,
            board: &deal.board[..len],
            history: &hist,
            seat,
        };
        let hole = deal.holes[seat];
        let action = if players[seat].exposes_probs() {
            let d = players[seat].dist(&view, hole);
            let i = d.sample(u);
            if let Some(t) = tracker.as_mut() {
                t.decision(&st, len, &d, i);
            }
            d.action(i)
        } else {
            players[seat].act(&view, hole, u)
        };
        let pre = st;
        let street = st.street();
        let step = st.apply(action);
        hist.push(Event {
            seat: seat as u8,
            street,
            action,
        });
        actions += 1;
        for p in players.iter_mut() {
            p.observe(&pre, seat, action);
        }
        let reveal = |to: usize,
                      st: &State,
                      len: &mut usize,
                      tracker: &mut Option<Tracker>,
                      players: &mut [Box<dyn crate::policy::Player>]| {
            if let Some(t) = tracker.as_mut() {
                t.chance(st, *len, to);
            }
            *len = to;
            for p in players.iter_mut() {
                p.observe_board(&deal.board[..to]);
            }
        };
        match step {
            Step::Continue => {}
            Step::NewStreet(s) => reveal(s.board_len(), &st, &mut len, &mut tracker, players),
            Step::Terminal => {
                if st.showdown() {
                    while len < 5 {
                        let to = if len == 0 { 3 } else { len + 1 };
                        reveal(to, &st, &mut len, &mut tracker, players);
                    }
                }
            }
        }
    }
    for p in players.iter_mut() {
        p.finish(deal, &hist, &st);
    }
    let payoffs = st.payoffs(deal);
    let aivat = tracker.map(|t| {
        // Fractional-chop realised value: the estimator at the terminal node (exact).
        let mut u = [0.0; MAX_PLAYERS];
        let q = crate::aivat::Query {
            state: &st,
            holes: &t.deal.holes,
            board: &t.deal.board[..len.max(if st.showdown() { 5 } else { 0 })],
        };
        let v = t.est.value(&q, t.sc);
        for i in 0..MAX_PLAYERS {
            u[i] = if st.showdown() || st.live_count() == 1 {
                v[i]
            } else {
                payoffs[i] as f64
            };
            u[i] += t.corr[i];
        }
        u
    });
    HandOutcome {
        payoffs,
        aivat,
        actions,
    }
}

#[derive(Clone, Debug, Default)]
pub struct SeatReport {
    pub raw: Est,
    pub aivat: Option<Est>,
}

#[derive(Clone, Debug)]
pub struct EntrantReport {
    pub name: String,
    pub raw: Est,
    pub aivat: Option<Est>,
    /// Indexed by seat; `None` if the entrant never sat there.
    pub by_seat: Vec<Option<SeatReport>>,
    /// Var(raw)/Var(aivat) per single hand (entrant-seat payoff) and per deal (duplicate mean).
    pub var_ratio_hand: Option<f64>,
    pub var_ratio_deal: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct MatchReport {
    pub entrants: Vec<EntrantReport>,
    pub deals: u64,
    pub hands: u64,
}

struct RotOut {
    raw: [f64; MAX_PLAYERS],
    av: Option<[f64; MAX_PLAYERS]>,
}

#[derive(Clone, Default)]
struct EntAcc {
    raw_deal: Acc,
    av_deal: Acc,
    raw_hand: Acc,
    av_hand: Acc,
    seat_raw: Vec<Acc>,
    seat_av: Vec<Acc>,
}

fn play_deal(spec: &MatchSpec, d: u64) -> Vec<RotOut> {
    let dseed = mix(spec.seed, d);
    let deal = Deal::random(&mut Rng::new(mix(dseed, 1)));
    let mut sc = Scratch::new(mix(dseed, 2));
    let n = spec.cfg.players;
    let mut out = Vec::new();
    for (r, seating) in spec.seatings.0.iter().enumerate() {
        let mut players: Vec<_> = (0..n)
            .map(|s| {
                spec.entrants[seating[s]].new_player_ctx(
                    s,
                    &spec.cfg,
                    mix(dseed, 100 + (r * MAX_PLAYERS + s) as u64),
                    &seating[..n],
                )
            })
            .collect();
        let mut crn = Rng::new(mix(dseed, if spec.crn { 3 } else { 1000 + r as u64 }));
        let tracker = match spec.aivat {
            AivatMode::Off => None,
            m => Some(Tracker::new(
                &*spec.estimator,
                &mut sc,
                if m == AivatMode::Full {
                    Mode::Full
                } else {
                    Mode::CardsOnly
                },
                deal,
            )),
        };
        let h = play_hand(&spec.cfg, &deal, &mut players, &mut crn, tracker);
        let mut raw = [0.0; MAX_PLAYERS];
        for s in 0..n {
            raw[s] = h.payoffs[s] as f64;
        }
        out.push(RotOut { raw, av: h.aivat });
    }
    out
}

pub fn run_match(spec: &MatchSpec) -> MatchReport {
    let ne = spec.entrants.len();
    let n = spec.cfg.players;
    let bb = spec.cfg.big_blind as f64;
    let use_av = spec.aivat != AivatMode::Off;
    let mut accs: Vec<EntAcc> = (0..ne)
        .map(|_| EntAcc {
            seat_raw: vec![Acc::default(); n],
            seat_av: vec![Acc::default(); n],
            ..Default::default()
        })
        .collect();
    let chunk = 1024u64;
    let mut start = 0;
    while start < spec.deals {
        let end = (start + chunk).min(spec.deals);
        let res: Vec<Vec<RotOut>> = (start..end)
            .into_par_iter()
            .map(|d| play_deal(spec, d))
            .collect();
        for rots in &res {
            for (e, acc) in accs.iter_mut().enumerate() {
                let (mut sr, mut sa, mut k) = (0.0, 0.0, 0.0);
                let mut seat_sum = vec![(0.0, 0.0, 0u32); n];
                for (r, ro) in rots.iter().enumerate() {
                    let seating = &spec.seatings.0[r];
                    let seats: Vec<usize> = (0..n).filter(|&s| seating[s] == e).collect();
                    if seats.is_empty() {
                        continue;
                    }
                    let m = seats.len() as f64;
                    let raw = seats.iter().map(|&s| ro.raw[s]).sum::<f64>() / m;
                    let av = ro
                        .av
                        .map(|a| seats.iter().map(|&s| a[s]).sum::<f64>() / m)
                        .unwrap_or(raw);
                    acc.raw_hand.add(raw);
                    acc.av_hand.add(av);
                    sr += raw;
                    sa += av;
                    k += 1.0;
                    for &s in &seats {
                        let x = &mut seat_sum[s];
                        x.0 += ro.raw[s];
                        x.1 += ro.av.map(|a| a[s]).unwrap_or(ro.raw[s]);
                        x.2 += 1;
                    }
                }
                if k > 0.0 {
                    acc.raw_deal.add(sr / k);
                    acc.av_deal.add(sa / k);
                }
                for s in 0..n {
                    let x = seat_sum[s];
                    if x.2 > 0 {
                        acc.seat_raw[s].add(x.0 / x.2 as f64);
                        acc.seat_av[s].add(x.1 / x.2 as f64);
                    }
                }
            }
        }
        start = end;
    }
    let entrants = accs
        .iter()
        .enumerate()
        .map(|(e, a)| EntrantReport {
            name: spec.entrants[e].name(),
            raw: Est::from_chips(&a.raw_deal, bb),
            aivat: use_av.then(|| Est::from_chips(&a.av_deal, bb)),
            by_seat: (0..n)
                .map(|s| {
                    (a.seat_raw[s].n > 0).then(|| SeatReport {
                        raw: Est::from_chips(&a.seat_raw[s], bb),
                        aivat: use_av.then(|| Est::from_chips(&a.seat_av[s], bb)),
                    })
                })
                .collect(),
            var_ratio_hand: use_av.then(|| a.raw_hand.var() / a.av_hand.var().max(1e-12)),
            var_ratio_deal: use_av.then(|| a.raw_deal.var() / a.av_deal.var().max(1e-12)),
        })
        .collect();
    MatchReport {
        entrants,
        deals: spec.deals,
        hands: spec.deals * spec.seatings.0.len() as u64,
    }
}

impl MatchReport {
    pub fn print(&self) {
        println!("{} deals, {} hands", self.deals, self.hands);
        for e in &self.entrants {
            print!("  {:<22} raw {} mbb/g", e.name, e.raw);
            if let (Some(a), Some(vr)) = (&e.aivat, e.var_ratio_hand) {
                print!(
                    "  aivat {a}  var-ratio hand {vr:.1}x deal {:.1}x",
                    e.var_ratio_deal.unwrap_or(0.0)
                );
            }
            println!();
            let seats: Vec<String> = e
                .by_seat
                .iter()
                .map(|s| {
                    s.as_ref()
                        .map_or("-".into(), |s| format!("{:+.0}", s.raw.mbb))
                })
                .collect();
            println!("    by seat (SB,BB,..): {}", seats.join(" "));
        }
    }
}
