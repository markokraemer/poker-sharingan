//! Composite full-game policy from nested unsafe search (supp. Algorithm 2), for exact
//! evaluation. Search is run once per round-root public state, in round order:
//!
//!  1. beliefs = reach under the *belief* profile (blueprint, then the previous search's
//!     AVERAGE strategy; Algorithm 2 line 3/9/19);
//!  2. solve the subgame (Linear CFR);
//!  3. write the FINAL iterate (or average, [`Play`]) into the played profile and the
//!     AVERAGE into the belief profile for the infosets of the root round.
//!
//! "Freeze own actions" (Algorithm 2 line 13) is vacuous here: Leduc has no off-tree
//! actions, so a round is searched exactly once from its root and a player's earlier
//! actions of the same round never need to be re-searched. Rounds before the root are
//! frozen by construction (their infosets are never touched).

use crate::beliefs;
use crate::poker::Poker;
use crate::subgame::{self, Conts, Ctx, Depth, Leaf};
use crate::view::View;
use cfr_lab::eval;
use cfr_lab::{Profile, Tree};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Play {
    Final,
    Average,
}

#[derive(Clone, Copy, Debug)]
pub struct SearchCfg {
    pub leaf: Leaf,
    pub depth: Depth,
    pub iters: u64,
    pub play: Play,
    /// Strategy used to update beliefs (Pluribus: Average).
    pub belief: Play,
    /// Search the first round's root (Pluribus mostly uses the blueprint preflop).
    pub round0: bool,
    pub round1: bool,
}

impl SearchCfg {
    /// Pluribus defaults: 4 continuations, depth-limited, play final, belief average.
    pub fn pluribus(iters: u64) -> Self {
        SearchCfg {
            leaf: Leaf::Cont4,
            depth: Depth::RoundEnd,
            iters,
            play: Play::Final,
            belief: Play::Average,
            round0: true,
            round1: true,
        }
    }
}

pub struct Composite {
    pub played: Profile,
    pub belief: Profile,
    pub subgames: usize,
}

pub struct Searcher<'a> {
    pub tree: &'a Tree,
    pub view: View,
    pub blueprint: Profile,
    conts: Conts,
}

impl<'a> Searcher<'a> {
    pub fn new<P: Poker>(tree: &'a Tree, game: &P, blueprint: &[f64]) -> Self {
        let kinds = subgame::action_kinds(tree, game);
        Searcher {
            tree,
            view: View::build(tree, game),
            blueprint: blueprint.to_vec(),
            conts: Conts::new(tree, &kinds, blueprint),
        }
    }

    pub fn compose(&self, cfg: &SearchCfg) -> Composite {
        let tree = self.tree;
        let cx = Ctx {
            tree,
            view: &self.view,
            conts: &self.conts,
        };
        let mut played = self.blueprint.clone();
        let mut belief = self.blueprint.clone();
        let mut subgames = 0;
        for round in 0..tree.n_rounds {
            if !(if round == 0 { cfg.round0 } else { cfg.round1 }) {
                continue;
            }
            let reach = beliefs::reach(tree, &belief);
            // Beliefs of this round are fixed before any of its subgames write back.
            let mut writes: Vec<(usize, Vec<f64>, Vec<f64>)> = vec![];
            let cuts = cfg.depth == Depth::RoundEnd && round + 1 < tree.n_rounds;
            let per_searcher = cfg.leaf == Leaf::Modicum && cuts;
            for (_, nodes) in self.view.round_roots(tree, round) {
                let roots = subgame::root_weights(&reach, &nodes);
                if roots.is_empty() {
                    continue;
                }
                let searchers: Vec<Option<usize>> = if per_searcher {
                    (0..tree.n_players).map(Some).collect()
                } else {
                    vec![None]
                };
                for s in searchers {
                    let sub = subgame::build(&cx, &roots, round, cfg.depth, cfg.leaf, s);
                    let (fin, avg) = subgame::solve(&sub, cfg.iters);
                    subgames += 1;
                    for (si, &f) in sub.full.iter().enumerate() {
                        if f == u32::MAX {
                            continue;
                        }
                        let info = &tree.info[f as usize];
                        if s.is_some_and(|s| info.player != s) || (cuts && info.round != round) {
                            continue;
                        }
                        let r = sub.tree.info[si].offset..sub.tree.info[si].offset + info.n_actions;
                        let pick = |m: Play| if m == Play::Final { &fin } else { &avg };
                        writes.push((
                            f as usize,
                            pick(cfg.play)[r.clone()].to_vec(),
                            pick(cfg.belief)[r].to_vec(),
                        ));
                    }
                }
            }
            for (f, p, b) in writes {
                let o = tree.info[f].offset;
                played[o..o + p.len()].copy_from_slice(&p);
                belief[o..o + b.len()].copy_from_slice(&b);
            }
        }
        Composite {
            played,
            belief,
            subgames,
        }
    }
}

/// Seat 0 plays `a`, seat 1 plays `b` (2-player).
pub fn mix(tree: &Tree, a: &[f64], b: &[f64]) -> Profile {
    let mut out = a.to_vec();
    for i in tree.info.iter().filter(|i| i.player == 1) {
        out[i.offset..i.offset + i.n_actions].copy_from_slice(&b[i.offset..i.offset + i.n_actions]);
    }
    out
}

/// Expected chips per hand of `a` against `b`, averaged over both seats (2-player).
pub fn head_to_head(tree: &Tree, a: &[f64], b: &[f64]) -> f64 {
    let v0 = eval::values(tree, &mix(tree, a, b))[0];
    let v1 = eval::values(tree, &mix(tree, b, a))[1];
    (v0 + v1) / 2.0
}
