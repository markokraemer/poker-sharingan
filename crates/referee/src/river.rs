//! Heads-up river referee: how many mbb does a policy's river strategy lose against the exact
//! equilibrium of the same spot?
//!
//! Pipeline for one spot (a PokerBench river row truncated to the river start):
//! 1. replay the history on the engine (6-max, all other seats folded);
//! 2. ranges: Bayes-update the policy's own action probabilities along the history for every
//!    hole pair ([`RangeMode::Policy`]) or use uniform ranges;
//! 3. solve the river subgame with CFR+ ([`crate::solver`]) over a fixed bet menu;
//! 4. query the policy at every river decision node for every hand in range, map its action
//!    distribution to the menu (nearest raise size), giving a full strategy;
//! 5. exact best responses give the policy's EV loss per seat, in mbb of the big blind
//!    (`value_p(eq) + BR_opponent(policy_p)`); a calling station is evaluated as a control.
//!
//! The policy is assumed to play both seats, so both ranges and both strategies come from it.

use crate::pokerbench::{replay_to_river, Spot};
use crate::replay::{feed, nearest_action, Replay, Step, BB};
use crate::solver::{Game, Kind, Strategy, N};
use bench::mix;
use bench::policy::{Dist, PolicyFactory, View};
use cards::{all_holes, Card};
use nlhe::abstraction::{BetSizing, Menu};
use nlhe::{Action, State};
use serde::Serialize;

/// River menu: first bet {0.5, 1.0} pot + all-in, later raises {1.0} + all-in, up to 3 raises
/// (the Pluribus river abstraction from the paper).
pub fn river_sizing() -> BetSizing {
    let a = [Menu::new(&[0.5, 1.0], true), Menu::new(&[1.0], true)];
    BetSizing::new([&a, &a, &a, &a], [3, 3, 3, 3])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeMode {
    /// Bayes ranges from the policy's own action probabilities.
    Policy,
    Uniform,
}

pub struct RiverSpot {
    pub replay: Replay,
    /// `[oop, ip]` seats.
    pub seats: [usize; 2],
    pub board: [Card; 5],
}

impl RiverSpot {
    pub fn from_spot(spot: &Spot) -> Result<RiverSpot, String> {
        let (replay, seats) = replay_to_river(spot)?;
        let board: [Card; 5] = replay.board.clone().try_into().map_err(|_| "board")?;
        Ok(RiverSpot {
            replay,
            seats,
            board,
        })
    }

    /// Key identifying the river node (many dataset rows share one).
    pub fn key(spot: &Spot) -> String {
        let (mut deals, mut cut) = (0, spot.post_line.len());
        for (i, t) in spot.post_line.iter().enumerate() {
            if *t == crate::pokerbench::PostTok::Deal {
                deals += 1;
                if deals == 2 {
                    cut = i;
                    break;
                }
            }
        }
        format!(
            "{}|{}|{:?}",
            crate::pokerbench::fmt_pre_line(&spot.pre_line),
            cards::cards_str(&spot.board_all),
            &spot.post_line[..cut]
        )
    }

    pub fn pot_bb(&self) -> f64 {
        self.replay.state.pot() as f64 / BB as f64
    }
}

fn floor(p: f64) -> f64 {
    p.max(1e-4)
}

/// Probability that a player with this `Dist` takes `taken` (raises mapped to the nearest size).
fn taken_prob(d: &Dist, pre: &State, taken: Action) -> f64 {
    let acts = d.actions();
    match nearest_action(pre, acts, taken) {
        Some(i) => d.prob(i),
        None => 0.0,
    }
}

/// Bayes range of `seat` over all hole pairs: product of the policy's probabilities of the
/// actions that seat actually took (normalised to max 1). Falls back to uniform if empty.
pub fn bayes_range(rs: &RiverSpot, f: &dyn PolicyFactory, player: usize, seed: u64) -> Vec<f64> {
    let seat = rs.seats[player];
    let cfg = rs.replay.cfg;
    let holes = all_holes();
    let bm = cards::mask_of(&rs.board);
    let mut w: Vec<f64> = holes
        .iter()
        .map(|h| {
            if cards::bit(h[0]) & bm == 0 && cards::bit(h[1]) & bm == 0 {
                1.0
            } else {
                0.0
            }
        })
        .collect();
    let mut p = f.new_player(seat, &cfg, seed);
    let mut k = 0;
    for s in &rs.replay.steps {
        match *s {
            Step::Act {
                pre,
                seat: s_seat,
                action,
                blen,
            } => {
                if s_seat == seat {
                    let v = View {
                        cfg: &cfg,
                        state: &pre,
                        board: &rs.board[..blen],
                        history: &rs.replay.events[..k],
                        seat,
                    };
                    let partial = cards::mask_of(&rs.board[..blen]);
                    for (i, h) in holes.iter().enumerate() {
                        if w[i] > 0.0
                            && cards::bit(h[0]) & partial == 0
                            && cards::bit(h[1]) & partial == 0
                        {
                            let d = p.dist(&v, *h);
                            w[i] *= floor(taken_prob(&d, &pre, action));
                        }
                    }
                }
                p.observe(&pre, s_seat, action);
                k += 1;
            }
            Step::Board(n) => p.observe_board(&rs.board[..n]),
        }
    }
    let mx = w.iter().cloned().fold(0.0, f64::max);
    if mx <= 0.0 {
        return vec![1.0; N];
    }
    w.iter().map(|x| x / mx).collect()
}

/// The policy's strategy on every river decision node of `game` (hands outside the range get
/// a uniform dummy).
pub fn policy_strategy(game: &Game, rs: &RiverSpot, f: &dyn PolicyFactory, seed: u64) -> Strategy {
    let holes = all_holes();
    let cfg = rs.replay.cfg;
    let mut out = vec![Vec::new(); game.tree.nodes.len()];
    for id in game.tree.action_nodes() {
        let node = &game.tree.nodes[id];
        let Kind::Action { player, acts, .. } = &node.kind else {
            unreachable!()
        };
        let seat = game.tree.seats[*player];
        let mut p = f.new_player(seat, &cfg, mix(seed, id as u64));
        feed(&mut *p, &rs.replay.steps, &rs.board);
        let mut events = rs.replay.events.clone();
        let mut st = rs.replay.state;
        for &a in &node.path {
            let s = st.to_act().unwrap();
            p.observe(&st, s, a);
            events.push(bench::policy::Event {
                seat: s as u8,
                street: st.street(),
                action: a,
            });
            st.apply(a);
        }
        let v = View {
            cfg: &cfg,
            state: &node.state,
            board: &rs.board,
            history: &events,
            seat,
        };
        let n = acts.len();
        let mut s = vec![0.0; n * N];
        for h in 0..N {
            if game.range[*player][h] <= 0.0 {
                for a in 0..n {
                    s[a * N + h] = 1.0 / n as f64;
                }
                continue;
            }
            let d = p.dist(&v, holes[h]);
            let mut tot = 0.0;
            for i in 0..d.len() {
                let j = nearest_action(&node.state, acts, d.action(i)).unwrap_or(0);
                let pr = d.prob(i);
                s[j * N + h] += pr;
                tot += pr;
            }
            for a in 0..n {
                s[a * N + h] = if tot > 0.0 {
                    s[a * N + h] / tot
                } else {
                    1.0 / n as f64
                };
            }
        }
        out[id] = s;
    }
    Strategy(out)
}

#[derive(Clone, Debug, Serialize)]
pub struct RiverResult {
    pub pot_bb: f64,
    pub stack_bb: f64,
    pub nodes: usize,
    /// Reference solver exploitability, mbb (solver error).
    pub ref_expl_mbb: f64,
    /// EV loss of the policy's river strategy per seat `[oop, ip]`, mbb.
    pub ev_loss_mbb: [f64; 2],
    /// Same for a calling station (control).
    pub station_loss_mbb: [f64; 2],
    /// Equilibrium value per seat, mbb.
    pub eq_value_mbb: [f64; 2],
}

/// Evaluate one spot. `iters`: CFR+ iterations for the reference equilibrium.
pub fn evaluate(
    rs: &RiverSpot,
    f: &dyn PolicyFactory,
    mode: RangeMode,
    iters: usize,
    seed: u64,
) -> Result<RiverResult, String> {
    let range = match mode {
        RangeMode::Policy => [
            bayes_range(rs, f, 0, mix(seed, 1)),
            bayes_range(rs, f, 1, mix(seed, 2)),
        ],
        RangeMode::Uniform => [vec![1.0; N], vec![1.0; N]],
    };
    let game = Game::new(
        rs.replay.state,
        rs.seats,
        rs.board,
        range,
        &river_sizing(),
        BB,
    )?;
    let eq = game.solve(iters);
    let pol = policy_strategy(&game, rs, f, mix(seed, 3));
    let station = station_strategy(&game);
    let bbm = 1000.0 / BB as f64;
    let v = game.value(&eq);
    let st = &rs.replay.state;
    Ok(RiverResult {
        pot_bb: st.pot() as f64 / BB as f64,
        stack_bb: st.stack(rs.seats[0]).min(st.stack(rs.seats[1])) as f64 / BB as f64,
        nodes: game.tree.nodes.len(),
        ref_expl_mbb: game.exploitability(&eq) * bbm,
        ev_loss_mbb: game.ev_loss(&pol, &eq),
        station_loss_mbb: game.ev_loss(&station, &eq),
        eq_value_mbb: [v[0] * bbm, v[1] * bbm],
    })
}

/// Always check/call.
pub fn station_strategy(game: &Game) -> Strategy {
    Strategy::from_fn(&game.tree, |id, a, _, _| {
        let Kind::Action { acts, .. } = &game.tree.nodes[id].kind else {
            unreachable!()
        };
        matches!(acts[a], Action::Check | Action::Call) as u8 as f64
    })
}
