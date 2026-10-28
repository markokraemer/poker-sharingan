//! Per-node annotation of a cfr-lab [`Tree`]: private cards, board, and the *public state*
//! (action path + board). The tree only stores infosets, but search needs "which nodes
//! share a public state" and "which private card does player p hold at node h".

use crate::poker::Poker;
use cfr_lab::tree::{Kind, Tree};
use std::collections::HashMap;

pub const MAXP: usize = 6;
pub const NONE: u8 = 255;

pub struct View {
    /// Private card per player at each node (`NONE` before it is dealt).
    pub cards: Vec<[u8; MAXP]>,
    pub board: Vec<u8>,
    /// Public-state id: (player-action index path, board card).
    pub pkey: Vec<u32>,
    /// Number of player actions taken so far.
    pub depth: Vec<u16>,
    pub parent: Vec<u32>,
    pub publics: Vec<(Vec<u8>, u8)>,
    lookup: HashMap<(Vec<u8>, u8), u32>,
}

struct Walk<'a> {
    t: &'a Tree,
    players: usize,
    deck: usize,
    v: View,
}

impl View {
    pub fn build<P: Poker>(tree: &Tree, game: &P) -> View {
        let n = tree.nodes.len();
        let v = View {
            cards: vec![[NONE; MAXP]; n],
            board: vec![NONE; n],
            pkey: vec![0; n],
            depth: vec![0; n],
            parent: vec![u32::MAX; n],
            publics: vec![],
            lookup: HashMap::new(),
        };
        let mut w = Walk {
            t: tree,
            players: game.players(),
            deck: game.deck(),
            v,
        };
        w.go(Tree::ROOT, [NONE; MAXP], NONE, &mut vec![]);
        w.v
    }

    pub fn lookup(&self, path: &[u8], board: Option<u8>) -> Option<u32> {
        self.lookup
            .get(&(path.to_vec(), board.unwrap_or(NONE)))
            .copied()
    }

    /// Round-start decision nodes of `round`, grouped by public state: the children of the
    /// board-dealing chance node (round >= 1), or of the private-deal chain (round 0).
    pub fn round_roots(&self, tree: &Tree, round: usize) -> Vec<(u32, Vec<u32>)> {
        let mut groups: Vec<(u32, Vec<u32>)> = vec![];
        let mut at: HashMap<u32, usize> = HashMap::new();
        for (id, nd) in tree.nodes.iter().enumerate() {
            if nd.kind == Kind::Chance || nd.round as usize != round || id == 0 {
                continue;
            }
            let par = &tree.nodes[self.parent[id] as usize];
            let start = par.kind == Kind::Chance
                && if round == 0 {
                    self.depth[id] == 0
                } else {
                    par.round as usize == round && self.depth[id] > 0
                };
            if start {
                let g = *at.entry(self.pkey[id]).or_insert_with(|| {
                    groups.push((self.pkey[id], vec![]));
                    groups.len() - 1
                });
                groups[g].1.push(id as u32);
            }
        }
        groups
    }
}

impl Walk<'_> {
    fn go(&mut self, id: u32, cards: [u8; MAXP], board: u8, path: &mut Vec<u8>) {
        let t = self.t;
        let key = (path.clone(), board);
        let next = self.v.publics.len() as u32;
        let pk = *self.v.lookup.entry(key.clone()).or_insert(next);
        if pk == next {
            self.v.publics.push(key);
        }
        let i = id as usize;
        self.v.cards[i] = cards;
        self.v.board[i] = board;
        self.v.pkey[i] = pk;
        self.v.depth[i] = path.len() as u16;
        let n = &t.nodes[i];
        for (a, &c) in t.kids(n).iter().enumerate() {
            self.v.parent[c as usize] = id;
            match n.kind {
                Kind::Terminal => {}
                Kind::Chance => {
                    let dealt = cards.iter().filter(|&&x| x != NONE).count();
                    let mut cs = cards;
                    let mut b = board;
                    let free = (0..self.deck as u8)
                        .filter(|x| !cards.contains(x) && *x != board)
                        .nth(a)
                        .unwrap();
                    if dealt < self.players {
                        cs[dealt] = free;
                    } else {
                        b = free;
                    }
                    self.go(c, cs, b, path);
                }
                Kind::Player => {
                    path.push(a as u8);
                    self.go(c, cards, board, path);
                    path.pop();
                }
            }
        }
    }
}
