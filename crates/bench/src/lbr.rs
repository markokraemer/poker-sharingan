//! Local Best Response (Lisý & Bowling 2017) against any exposing, forkable policy.
//!
//! The LBR player tracks each opponent's range by Bayes: after every opponent decision the
//! reach weight of each of the 1326 hole pairs is multiplied by the policy's probability of
//! that action with those cards (`Player::dist` queried for hypothetical holes). At its own
//! decisions it evaluates {fold, call/check, raise 0.5 pot, raise 1 pot, all-in} by Monte
//! Carlo over opponent holes drawn from the posterior and board run-outs, assuming that after
//! the action all remaining live players check/call down to showdown. Opponent responses to a
//! raise use the policy's own fold probability for the sampled hole (computed on a forked
//! player, sequentially for multiple opponents; a re-raise is treated as a call). The chosen
//! action is actually played and the real chip winnings are measured, so the reported
//! mbb/game is a genuine lower bound on the target's exploitability (heads-up) -- LBR is
//! a weak exploiter, so it understates it.
//!
//! `Mode::Hu` (2 players): LBR sits in each seat on every deal. `Mode::Profile` (n players):
//! LBR sits in each seat against n-1 copies of the policy. The profile variant measures
//! exploitability of the policy *profile* by one deviator and is NOT an equilibrium-gap bound
//! for n > 2 players.

use crate::mix;
use crate::policy::{Event, Player, PolicyFactory, View};
use crate::stats::{Acc, Est};
use cards::{all_holes, bit, evaluate, hole_index, mask_of, Card, Rng, FULL_DECK};
use nlhe::{Action, Deal, GameConfig, State, Step, MAX_PLAYERS};
use rayon::prelude::*;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct LbrConfig {
    /// Monte Carlo samples per decision.
    pub samples: usize,
    /// Raise sizes as pot fractions (all-in is always added).
    pub fracs: Vec<f32>,
}

impl Default for LbrConfig {
    fn default() -> Self {
        LbrConfig {
            samples: 300,
            fracs: vec![0.5, 1.0],
        }
    }
}

#[derive(Clone, Debug)]
pub struct LbrReport {
    pub name: String,
    pub lbr: Est,
    pub by_seat: Vec<Est>,
    pub deals: u64,
}

impl std::fmt::Display for LbrReport {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "LBR vs {:<22} {} mbb/g  (seats: ", self.name, self.lbr)?;
        for s in &self.by_seat {
            write!(f, "{:+.0} ", s.mbb)?;
        }
        write!(f, ")")
    }
}

/// Gross fractional-chop pot shares given per-seat totals, live mask and ranks.
fn settle(
    n: usize,
    totals: &[i32; MAX_PLAYERS],
    live: u8,
    ranks: &[u32; MAX_PLAYERS],
) -> [f64; MAX_PLAYERS] {
    let mut out = [0.0; MAX_PLAYERS];
    let mut prev = 0;
    loop {
        let Some(lv) = (0..n)
            .filter(|&i| live >> i & 1 != 0 && totals[i] > prev)
            .map(|i| totals[i])
            .min()
        else {
            break;
        };
        let pot: i32 = (0..n)
            .map(|i| totals[i].min(lv) - totals[i].min(prev))
            .sum();
        let elig: Vec<usize> = (0..n)
            .filter(|&i| live >> i & 1 != 0 && totals[i] >= lv)
            .collect();
        let best = elig.iter().map(|&i| ranks[i]).max().unwrap();
        let w: Vec<usize> = elig.into_iter().filter(|&i| ranks[i] == best).collect();
        for &i in &w {
            out[i] += pot as f64 / w.len() as f64;
        }
        prev = lv;
    }
    out
}

/// Cumulative distribution over hole indices (zero weight on dead cards).
fn cum_weights(w: &[f32], dead: u64) -> Vec<f64> {
    let hs = all_holes();
    let mut acc = 0.0;
    hs.iter()
        .enumerate()
        .map(|(i, h)| {
            if bit(h[0]) & dead == 0 && bit(h[1]) & dead == 0 {
                acc += w[i] as f64;
            }
            acc
        })
        .collect()
}

fn sample_hole(cum: &[f64], rng: &mut Rng) -> Option<[Card; 2]> {
    let total = *cum.last().unwrap();
    if total <= 0.0 {
        return None;
    }
    let x = rng.f64() * total;
    let i = cum.partition_point(|&c| c <= x).min(cum.len() - 1);
    Some(all_holes()[i])
}

struct Option_ {
    action: Action,
    /// For raises: per chained opponent seat, fold probability by hole index.
    fold: Vec<(usize, Vec<f32>)>,
}

#[allow(clippy::too_many_arguments)]
fn decide(
    cfg: &GameConfig,
    cfgl: &LbrConfig,
    st: &State,
    me: usize,
    my_hole: [Card; 2],
    board: &[Card],
    hist: &[Event],
    players: &[Option<Box<dyn Player>>],
    w: &[Vec<f32>],
    rng: &mut Rng,
) -> Action {
    let n = cfg.players;
    let l = st.legal();
    let passive = if l.can_check {
        Action::Check
    } else {
        Action::Call
    };
    let mut opts: Vec<Option_> = Vec::new();
    if l.can_fold {
        opts.push(Option_ {
            action: Action::Fold,
            fold: vec![],
        });
    }
    opts.push(Option_ {
        action: passive,
        fold: vec![],
    });
    if l.can_raise {
        let mut tos: Vec<i32> = cfgl
            .fracs
            .iter()
            .map(|f| {
                (st.cur_bet() + (f * (st.pot() + l.call_amount) as f32).round() as i32)
                    .clamp(l.min_raise_to, l.max_raise_to)
            })
            .collect();
        tos.push(l.max_raise_to);
        tos.sort_unstable();
        tos.dedup();
        for to in tos {
            let a = Action::RaiseTo(to);
            // Opponent fold probabilities on forks, chaining calls through the round.
            let mut forks: Vec<Option<Box<dyn Player>>> = players
                .iter()
                .map(|p| {
                    p.as_ref()
                        .map(|p| p.fork().expect("LBR target must support fork()"))
                })
                .collect();
            let mut h2 = hist.to_vec();
            let mut s2 = *st;
            let street = st.street();
            let mut act = a;
            let mut actor = me;
            let mut fold = Vec::new();
            loop {
                let pre = s2;
                h2.push(Event {
                    seat: actor as u8,
                    street: s2.street(),
                    action: act,
                });
                let step = s2.apply(act);
                for f in forks.iter_mut().flatten() {
                    f.observe(&pre, actor, act);
                }
                if step != Step::Continue {
                    break;
                }
                let j = s2.to_act().unwrap();
                if j == me {
                    break;
                }
                let view = View {
                    cfg,
                    state: &s2,
                    board,
                    history: &h2,
                    seat: j,
                };
                let mut fp = vec![0f32; 1326];
                for (i, hc) in all_holes().iter().enumerate() {
                    if (bit(hc[0]) | bit(hc[1]))
                        & (mask_of(board) | bit(my_hole[0]) | bit(my_hole[1]))
                        == 0
                    {
                        fp[i] = forks[j]
                            .as_mut()
                            .unwrap()
                            .dist(&view, *hc)
                            .prob_of(Action::Fold) as f32;
                    }
                }
                fold.push((j, fp));
                actor = j;
                act = Action::Call;
                if s2.street() != street {
                    break;
                }
            }
            opts.push(Option_ { action: a, fold });
        }
    }

    // Monte Carlo evaluation with common samples across options.
    let dead0 = mask_of(board) | bit(my_hole[0]) | bit(my_hole[1]);
    let opp: Vec<usize> = (0..n).filter(|&j| j != me && !st.is_folded(j)).collect();
    let cums: Vec<Vec<f64>> = opp.iter().map(|&j| cum_weights(&w[j], dead0)).collect();
    let mut ev = vec![0.0f64; opts.len()];
    let mut got = 0usize;
    let totals0: [i32; MAX_PLAYERS] =
        std::array::from_fn(|i| if i < n { st.contributed(i) } else { 0 });
    let live0 = (0..n)
        .filter(|&i| !st.is_folded(i))
        .fold(0u8, |m, i| m | 1 << i);
    for _ in 0..cfgl.samples {
        let mut dead = dead0;
        let mut holes = [[0u8; 2]; MAX_PLAYERS];
        holes[me] = my_hole;
        let mut ok = true;
        for (k, &j) in opp.iter().enumerate() {
            let mut tries = 0;
            loop {
                match sample_hole(&cums[k], rng) {
                    Some(h) if bit(h[0]) & dead == 0 && bit(h[1]) & dead == 0 => {
                        holes[j] = h;
                        dead |= bit(h[0]) | bit(h[1]);
                        break;
                    }
                    _ => {
                        tries += 1;
                        if tries > 30 {
                            ok = false;
                            break;
                        }
                    }
                }
            }
            if !ok {
                break;
            }
        }
        if !ok {
            continue;
        }
        let mut run = mask_of(board);
        for _ in 0..5 - board.len() {
            run |= bit(rng.draw(dead | run));
        }
        let mut ranks = [0u32; MAX_PLAYERS];
        for i in 0..n {
            if live0 >> i & 1 != 0 {
                ranks[i] = evaluate(bit(holes[i][0]) | bit(holes[i][1]) | run);
            }
        }
        for (oi, o) in opts.iter().enumerate() {
            let mut totals = totals0;
            let mut live = live0;
            let v = match o.action {
                Action::Fold => -(totals0[me] as f64),
                Action::Check | Action::Call => {
                    totals[me] += l.call_amount;
                    settle(n, &totals, live, &ranks)[me] - totals[me] as f64
                }
                Action::RaiseTo(to) => {
                    totals[me] += to - st.street_bet(me);
                    for (j, fp) in &o.fold {
                        if rng.f32() < fp[hole_index(holes[*j][0], holes[*j][1])] {
                            live &= !(1 << j);
                        } else {
                            totals[*j] += (to - st.street_bet(*j)).min(st.stack(*j)).max(0);
                        }
                    }
                    if live.count_ones() == 1 {
                        totals.iter().take(n).sum::<i32>() as f64 - totals[me] as f64
                    } else {
                        settle(n, &totals, live, &ranks)[me] - totals[me] as f64
                    }
                }
            };
            ev[oi] += v;
        }
        got += 1;
    }
    if got == 0 {
        return passive;
    }
    let best = (0..opts.len()).fold(0, |b, i| if ev[i] > ev[b] + 1e-9 { i } else { b });
    // Prefer the passive option on exact ties (index of passive is 0 or 1).
    let pi = opts.iter().position(|o| o.action == passive).unwrap();
    if ev[best] <= ev[pi] + 1e-9 {
        return passive;
    }
    opts[best].action
}

/// Play one hand with LBR in `me` against `target` copies elsewhere; returns LBR's payoff.
pub fn lbr_hand(
    cfg: &GameConfig,
    cfgl: &LbrConfig,
    target: &dyn PolicyFactory,
    deal: &Deal,
    me: usize,
    seed: u64,
) -> i32 {
    let n = cfg.players;
    let mut rng = Rng::new(seed);
    let mut players: Vec<Option<Box<dyn Player>>> = (0..n)
        .map(|s| (s != me).then(|| target.new_player(s, cfg, mix(seed, s as u64))))
        .collect();
    assert!(
        players.iter().flatten().all(|p| p.exposes_probs()),
        "LBR target must expose probabilities"
    );
    let mut w: Vec<Vec<f32>> = vec![vec![1.0; 1326]; n];
    let mut st = State::new(cfg);
    let mut hist: Vec<Event> = Vec::new();
    let mut len = 0;
    let holes_all = all_holes();
    while !st.is_terminal() {
        let seat = st.to_act().unwrap();
        let board = &deal.board[..len];
        let action = if seat == me {
            decide(
                cfg,
                cfgl,
                &st,
                me,
                deal.holes[me],
                board,
                &hist,
                &players,
                &w,
                &mut rng,
            )
        } else {
            let view = View {
                cfg,
                state: &st,
                board,
                history: &hist,
                seat,
            };
            let p = players[seat].as_mut().unwrap();
            let d = p.dist(&view, deal.holes[seat]);
            let a = d.action(d.sample(rng.f64()));
            let dead = mask_of(board) | bit(deal.holes[me][0]) | bit(deal.holes[me][1]);
            for (i, hc) in holes_all.iter().enumerate() {
                if (bit(hc[0]) | bit(hc[1])) & dead == 0 {
                    w[seat][i] *= p.dist(&view, *hc).prob_of(a) as f32;
                }
            }
            a
        };
        let pre = st;
        let street = st.street();
        let step = st.apply(action);
        hist.push(Event {
            seat: seat as u8,
            street,
            action,
        });
        for p in players.iter_mut().flatten() {
            p.observe(&pre, seat, action);
        }
        let reveal = |to: usize, players: &mut Vec<Option<Box<dyn Player>>>, len: &mut usize| {
            *len = to;
            for p in players.iter_mut().flatten() {
                p.observe_board(&deal.board[..to]);
            }
        };
        match step {
            Step::NewStreet(s) => reveal(s.board_len(), &mut players, &mut len),
            Step::Terminal if st.showdown() => reveal(5, &mut players, &mut len),
            _ => {}
        }
    }
    st.payoffs(deal)[me]
}

/// Run LBR over `deals` random deals, with LBR in every seat on every deal.
pub fn run_lbr(
    cfg: &GameConfig,
    cfgl: &LbrConfig,
    target: Arc<dyn PolicyFactory>,
    deals: u64,
    seed: u64,
) -> LbrReport {
    let n = cfg.players;
    let res: Vec<Vec<i32>> = (0..deals)
        .into_par_iter()
        .map(|d| {
            let ds = mix(seed, d);
            let deal = Deal::random(&mut Rng::new(mix(ds, 1)));
            (0..n)
                .map(|me| lbr_hand(cfg, cfgl, &*target, &deal, me, mix(ds, 10 + me as u64)))
                .collect()
        })
        .collect();
    let bb = cfg.big_blind as f64;
    let (mut all, mut seat) = (Acc::default(), vec![Acc::default(); n]);
    for r in &res {
        all.add(r.iter().sum::<i32>() as f64 / n as f64);
        for (s, &x) in r.iter().enumerate() {
            seat[s].add(x as f64);
        }
    }
    let _ = FULL_DECK;
    LbrReport {
        name: target.name(),
        lbr: Est::from_chips(&all, bb),
        by_seat: seat.iter().map(|a| Est::from_chips(a, bb)).collect(),
        deals,
    }
}
