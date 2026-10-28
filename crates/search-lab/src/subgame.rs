//! Subgame construction and solving (supp.: "Structure of imperfect-information subgames",
//! "Leaf node values in imperfect-information subgames").
//!
//! A subgame is rebuilt as a fresh cfr-lab `Tree`:
//!  * root = chance node over the root public state's nodes, probabilities proportional to
//!    the joint belief (reach under the belief profile; card removal is implicit);
//!  * below it the real betting tree of the current round (infosets shared with the real
//!    tree via `Sub::full`);
//!  * depth limit [`Depth::RoundEnd`]: the next round's chance node is a *leaf*. There each
//!    player still in the hand picks one of 4 continuation strategies (blueprint, fold x5,
//!    call x5, raise x5, renormalised) as an extra decision node whose infoset is that
//!    player's infoset at the leaf. Choosers move sequentially but infosets exclude the
//!    others' choices (= simultaneous). The value of a joint choice is the exact expected
//!    value of the rest of the game when everyone follows their chosen continuation.
//!  * [`Leaf::Modicum`]: only the opponents of the searcher choose (Modicum, Brown et al.
//!    2018); the searcher is fixed to the blueprint. [`Leaf::Blueprint`]: nobody chooses.

use crate::beliefs;
use crate::poker::{ActKind, Poker};
use crate::view::{View, MAXP};
use cfr_lab::cfr::{Cfr, Variant};
use cfr_lab::tree::{Info, Kind, Node};
use cfr_lab::{Profile, Solver, Tree};
use std::collections::HashMap;

pub const NCONT: usize = 4;
pub const BIAS: f64 = 5.0;
const NO_FULL: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Depth {
    /// Leaves at the start of the next round.
    RoundEnd,
    /// Search to the end of the game.
    Game,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Leaf {
    /// Every remaining player chooses among 4 continuations (Pluribus).
    Cont4,
    /// Only the searcher's opponents choose; searcher = blueprint (Modicum).
    Modicum,
    /// Everyone plays the blueprint after the leaf (the "plain" baseline).
    Blueprint,
}

/// The 4 continuation profiles (index 0 = blueprint, 1 fold, 2 call, 3 raise biased).
pub struct Conts {
    pub profs: [Profile; NCONT],
}

impl Conts {
    pub fn new(tree: &Tree, kinds: &[Vec<ActKind>], bp: &[f64]) -> Conts {
        let mk = |bias: Option<ActKind>| {
            let mut out = bp.to_vec();
            if let Some(k) = bias {
                for (i, info) in tree.info.iter().enumerate() {
                    let s = &mut out[info.offset..info.offset + info.n_actions];
                    for (x, &kk) in s.iter_mut().zip(&kinds[i]) {
                        if kk == k {
                            *x *= BIAS;
                        }
                    }
                    let tot: f64 = s.iter().sum();
                    s.iter_mut().for_each(|x| *x /= tot);
                }
            }
            out
        };
        Conts {
            profs: [
                mk(None),
                mk(Some(ActKind::Fold)),
                mk(Some(ActKind::Call)),
                mk(Some(ActKind::Raise)),
            ],
        }
    }
}

pub fn action_kinds<P: Poker>(tree: &Tree, game: &P) -> Vec<Vec<ActKind>> {
    tree.info
        .iter()
        .map(|i| game.action_kinds(&i.key, i.n_actions))
        .collect()
}

pub struct Ctx<'a> {
    pub tree: &'a Tree,
    pub view: &'a View,
    pub conts: &'a Conts,
}

impl Ctx<'_> {
    /// Expected utilities from `id` when player p follows continuation `choice[p]`.
    pub fn eval_cont(&self, id: u32, choice: &[usize; MAXP], w: f64, out: &mut [f64]) {
        let t = self.tree;
        let n = &t.nodes[id as usize];
        match n.kind {
            Kind::Terminal => (0..t.n_players).for_each(|p| out[p] += w * t.utility(n, p)),
            Kind::Chance => {
                for (&c, &pr) in t.kids(n).iter().zip(t.kid_probs(n)) {
                    self.eval_cont(c, choice, w * pr, out);
                }
            }
            Kind::Player => {
                let off = t.info[n.infoset as usize].offset;
                let prof = &self.conts.profs[choice[n.player as usize]];
                for (a, &c) in t.kids(n).iter().enumerate() {
                    self.eval_cont(c, choice, w * prof[off + a], out);
                }
            }
        }
    }
}

/// A subgame as a standalone tree; `full[i]` = real infoset of subgame infoset i
/// (`u32::MAX` for leaf-choice infosets).
pub struct Sub {
    pub tree: Tree,
    pub full: Vec<u32>,
    pub leaves: usize,
}

struct B<'a> {
    cx: &'a Ctx<'a>,
    t: Tree,
    full: Vec<u32>,
    intern: HashMap<u32, u32>,
    leaf_info: HashMap<(usize, u8, u32), u32>,
    round: usize,
    depth: Depth,
    leaf: Leaf,
    searcher: Option<usize>,
    leaves: usize,
}

impl B<'_> {
    fn push(&mut self, kind: Kind, player: u8, round: u8, active: u32, infoset: u32) -> u32 {
        self.t.nodes.push(Node {
            kind,
            player,
            round,
            active,
            infoset,
            child_start: 0,
            n: 0,
            util: 0,
        });
        self.t.nodes.len() as u32 - 1
    }
    fn set_kids(&mut self, id: u32, kids: Vec<u32>, probs: Vec<f64>) {
        let start = self.t.children.len() as u32;
        let n = &mut self.t.nodes[id as usize];
        n.child_start = start;
        n.n = kids.len() as u32;
        self.t.children.extend(kids);
        self.t.probs.extend(probs);
    }
    fn terminal(&mut self, round: u8, active: u32, u: &[f64]) -> u32 {
        let id = self.push(Kind::Terminal, 0, round, active, 0);
        self.t.nodes[id as usize].util = self.t.utils.len() as u32;
        self.t.utils.extend_from_slice(u);
        id
    }
    fn new_info(&mut self, player: usize, n: usize, round: usize, key: String, full: u32) -> u32 {
        self.t.info.push(Info {
            player,
            n_actions: n,
            offset: self.t.total_actions,
            round,
            key,
        });
        self.t.total_actions += n;
        self.full.push(full);
        self.t.info.len() as u32 - 1
    }

    fn add(&mut self, f: u32) -> u32 {
        let tree = self.cx.tree;
        let n = &tree.nodes[f as usize];
        match n.kind {
            Kind::Terminal => {
                let u = tree.utils[n.util as usize..n.util as usize + tree.n_players].to_vec();
                self.terminal(n.round, n.active, &u)
            }
            Kind::Chance => {
                if self.depth == Depth::RoundEnd && n.round as usize > self.round {
                    return self.add_leaf(f);
                }
                let id = self.push(Kind::Chance, 0, n.round, n.active, 0);
                let kids = tree.kids(n).iter().map(|&c| self.add(c)).collect();
                self.set_kids(id, kids, tree.kid_probs(n).to_vec());
                id
            }
            Kind::Player => {
                let fi = n.infoset;
                let si = match self.intern.get(&fi) {
                    Some(&s) => s,
                    None => {
                        let i = &tree.info[fi as usize];
                        let s = self.new_info(i.player, i.n_actions, i.round, i.key.clone(), fi);
                        self.intern.insert(fi, s);
                        s
                    }
                };
                let id = self.push(Kind::Player, n.player, n.round, n.active, si);
                let kids = tree.kids(n).iter().map(|&c| self.add(c)).collect();
                self.set_kids(id, kids, vec![1.0; n.n as usize]);
                id
            }
        }
    }

    fn add_leaf(&mut self, f: u32) -> u32 {
        self.leaves += 1;
        let cx = self.cx;
        let n = &cx.tree.nodes[f as usize];
        let choosers: Vec<usize> = (0..cx.tree.n_players)
            .filter(|&p| n.active >> p & 1 == 1)
            .filter(|&p| match self.leaf {
                Leaf::Cont4 => true,
                Leaf::Modicum => Some(p) != self.searcher,
                Leaf::Blueprint => false,
            })
            .collect();
        self.chain(f, &choosers, 0, &mut [0; MAXP])
    }

    fn chain(&mut self, f: u32, ch: &[usize], i: usize, choice: &mut [usize; MAXP]) -> u32 {
        let cx = self.cx;
        let n = &cx.tree.nodes[f as usize];
        if i == ch.len() {
            let mut u = vec![0.0; cx.tree.n_players];
            cx.eval_cont(f, choice, 1.0, &mut u);
            return self.terminal(n.round, n.active, &u);
        }
        let p = ch[i];
        let card = cx.view.cards[f as usize][p];
        let pk = cx.view.pkey[f as usize];
        let si = match self.leaf_info.get(&(p, card, pk)) {
            Some(&s) => s,
            None => {
                let key = format!("L|{p}|{card}|{pk}");
                let s = self.new_info(p, NCONT, self.round, key, NO_FULL);
                self.leaf_info.insert((p, card, pk), s);
                s
            }
        };
        let id = self.push(Kind::Player, p as u8, n.round, n.active, si);
        let mut kids = vec![];
        for c in 0..NCONT {
            choice[p] = c;
            kids.push(self.chain(f, ch, i + 1, choice));
        }
        choice[p] = 0;
        self.set_kids(id, kids, vec![1.0; NCONT]);
        id
    }
}

/// Build the subgame rooted at a chance node over `roots` = (real node, weight).
pub fn build(
    cx: &Ctx,
    roots: &[(u32, f64)],
    round: usize,
    depth: Depth,
    leaf: Leaf,
    searcher: Option<usize>,
) -> Sub {
    let t = cx.tree;
    let mut b = B {
        cx,
        t: Tree {
            n_players: t.n_players,
            n_rounds: t.n_rounds,
            nodes: vec![],
            children: vec![],
            probs: vec![],
            utils: vec![],
            info: vec![],
            total_actions: 0,
        },
        full: vec![],
        intern: HashMap::new(),
        leaf_info: HashMap::new(),
        round,
        depth,
        leaf,
        searcher,
        leaves: 0,
    };
    let all = (1u32 << t.n_players) - 1;
    let root = b.push(Kind::Chance, 0, round as u8, all, 0);
    let tot: f64 = roots.iter().map(|r| r.1).sum();
    let kids = roots.iter().map(|&(n, _)| b.add(n)).collect();
    b.set_kids(root, kids, roots.iter().map(|r| r.1 / tot).collect());
    Sub {
        tree: b.t,
        full: b.full,
        leaves: b.leaves,
    }
}

/// Linear CFR for `iters` iterations; returns (final iterate, weighted average).
pub fn solve(sub: &Sub, iters: u64) -> (Profile, Profile) {
    let mut s = Cfr::new(&sub.tree, Variant::Linear);
    for _ in 0..iters {
        s.iterate();
    }
    (s.current(), s.average())
}

/// Root weights for a group of real nodes from the belief profile's reach.
pub fn root_weights(reach: &[f64], nodes: &[u32]) -> Vec<(u32, f64)> {
    nodes
        .iter()
        .map(|&n| (n, reach[n as usize]))
        .filter(|r| r.1 > 0.0)
        .collect()
}

pub fn belief_reach(tree: &Tree, belief: &[f64]) -> Vec<f64> {
    beliefs::reach(tree, belief)
}
