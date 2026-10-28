//! Flat game-tree arena built from a [`Game`], with interned information sets.
//!
//! A *profile* is a flat `Vec<f64>`: for infoset `i`, `profile[info[i].offset..][..n_actions]`
//! is the behaviour strategy (probabilities summing to one).

use crate::game::{Game, Turn};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Terminal,
    Chance,
    Player,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub kind: Kind,
    pub player: u8,
    pub round: u8,
    /// Bitmask of players still in the hand.
    pub active: u32,
    /// Infoset id (player nodes only).
    pub infoset: u32,
    pub child_start: u32,
    pub n: u32,
    /// Index into `Tree::utils` (terminal nodes only).
    pub util: u32,
}

#[derive(Clone, Debug)]
pub struct Info {
    pub player: usize,
    pub n_actions: usize,
    pub offset: usize,
    pub round: usize,
    pub key: String,
}

pub type Profile = Vec<f64>;

pub struct Tree {
    pub n_players: usize,
    pub n_rounds: usize,
    pub nodes: Vec<Node>,
    pub children: Vec<u32>,
    /// Chance probabilities parallel to `children` (1.0 for non-chance parents).
    pub probs: Vec<f64>,
    pub utils: Vec<f64>,
    pub info: Vec<Info>,
    pub total_actions: usize,
}

impl Tree {
    pub const ROOT: u32 = 0;

    pub fn build<G: Game>(g: &G) -> Tree {
        let mut t = Tree {
            n_players: g.num_players(),
            n_rounds: g.num_rounds(),
            nodes: vec![],
            children: vec![],
            probs: vec![],
            utils: vec![],
            info: vec![],
            total_actions: 0,
        };
        let mut intern = HashMap::new();
        t.add(g, &g.root(), &mut intern);
        t
    }

    fn add<G: Game>(
        &mut self,
        g: &G,
        s: &G::State,
        intern: &mut HashMap<(usize, String), u32>,
    ) -> u32 {
        let id = self.nodes.len() as u32;
        let turn = g.turn(s);
        let mut active = 0u32;
        for p in 0..self.n_players {
            if g.in_hand(s, p) {
                active |= 1 << p;
            }
        }
        let mut node = Node {
            kind: Kind::Terminal,
            player: 0,
            round: g.round(s) as u8,
            active,
            infoset: 0,
            child_start: 0,
            n: 0,
            util: 0,
        };
        match turn {
            Turn::Terminal => {
                node.util = self.utils.len() as u32;
                self.utils.extend(g.utilities(s));
                self.nodes.push(node);
                return id;
            }
            Turn::Chance => node.kind = Kind::Chance,
            Turn::Player(p) => {
                node.kind = Kind::Player;
                node.player = p as u8;
                let n = g.num_outcomes(s);
                let key = (p, g.info_key(s));
                let next = self.info.len() as u32;
                let iid = *intern.entry(key.clone()).or_insert(next);
                if iid == next {
                    self.info.push(Info {
                        player: p,
                        n_actions: n,
                        offset: self.total_actions,
                        round: node.round as usize,
                        key: key.1,
                    });
                    self.total_actions += n;
                }
                assert_eq!(
                    self.info[iid as usize].n_actions, n,
                    "infoset action count mismatch"
                );
                node.infoset = iid;
            }
        }
        let n = g.num_outcomes(s);
        node.n = n as u32;
        self.nodes.push(node);
        let probs = if turn == Turn::Chance {
            g.chance_probs(s)
        } else {
            vec![1.0; n]
        };
        let kids: Vec<u32> = (0..n)
            .map(|a| self.add(g, &g.child(s, a), intern))
            .collect();
        let start = self.children.len() as u32;
        self.children.extend(kids);
        self.probs.extend(probs);
        let nd = &mut self.nodes[id as usize];
        nd.child_start = start;
        id
    }

    #[inline]
    pub fn kids(&self, n: &Node) -> &[u32] {
        &self.children[n.child_start as usize..(n.child_start + n.n) as usize]
    }

    #[inline]
    pub fn kid_probs(&self, n: &Node) -> &[f64] {
        &self.probs[n.child_start as usize..(n.child_start + n.n) as usize]
    }

    #[inline]
    pub fn utility(&self, n: &Node, p: usize) -> f64 {
        self.utils[n.util as usize + p]
    }

    pub fn uniform_profile(&self) -> Profile {
        let mut v = vec![0.0; self.total_actions];
        for i in &self.info {
            v[i.offset..i.offset + i.n_actions].fill(1.0 / i.n_actions as f64);
        }
        v
    }

    pub fn infosets_of(&self, p: usize) -> usize {
        self.info.iter().filter(|i| i.player == p).count()
    }

    /// Normalise per-infoset non-negative weights into a profile (uniform where all zero).
    pub fn normalise(&self, w: &[f64]) -> Profile {
        let mut out = vec![0.0; self.total_actions];
        for i in &self.info {
            let s = &w[i.offset..i.offset + i.n_actions];
            let tot: f64 = s.iter().sum();
            for (o, x) in out[i.offset..i.offset + i.n_actions].iter_mut().zip(s) {
                *o = if tot > 0.0 {
                    x / tot
                } else {
                    1.0 / i.n_actions as f64
                };
            }
        }
        out
    }
}
