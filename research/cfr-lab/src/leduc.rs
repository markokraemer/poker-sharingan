//! Leduc hold'em with `n` players and a deck of `ranks` ranks x 2 suits.
//!
//! Standard rules (Southey et al. 2005, OpenSpiel): ante 1, one private card each, round 1
//! raise size 2, then one public card, round 2 raise size 4, at most 2 raises per round,
//! fold only legal when facing a bet. Showdown: pair with the public card beats high card;
//! ties split. 2 players with 3 ranks (6 cards) has 936 infosets (cards identified exactly,
//! including suit, as in the literature).
//!
//! The 3-player instance in this crate uses the same 6-card deck (3 ranks x 2 suits): 3
//! private cards + 1 public card still fit, and the tree stays small (8 cards would give
//! ~4x more deals). Pass `ranks` explicitly for larger decks.

use crate::game::{Game, Turn};

pub struct Leduc {
    pub players: usize,
    pub ranks: usize,
}

impl Leduc {
    pub fn new(players: usize, ranks: usize) -> Self {
        assert!((2..=6).contains(&players) && ranks * 2 > players);
        Leduc { players, ranks }
    }
    /// Standard 2-player, 6-card Leduc.
    pub fn standard() -> Self {
        Self::new(2, 3)
    }
}

const MAX_RAISES: u8 = 2;

#[derive(Clone, Debug)]
pub struct LeducState {
    cards: Vec<u8>,
    public: Option<u8>,
    round: u8,
    hist: Vec<u8>,
    contrib: Vec<i32>,
    round_contrib: Vec<i32>,
    folded: u32,
    to_act: usize,
    pending: usize,
    raises: u8,
    level: i32,
    need_public: bool,
    done: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Act {
    Fold,
    Call,
    Raise,
}

impl LeducState {
    fn legal(&self) -> ([Act; 3], usize) {
        let mut v = [Act::Call; 3];
        let mut n = 0;
        if self.level > self.round_contrib[self.to_act] {
            v[n] = Act::Fold;
            n += 1;
        }
        v[n] = Act::Call;
        n += 1;
        if self.raises < MAX_RAISES {
            v[n] = Act::Raise;
            n += 1;
        }
        (v, n)
    }
    fn active(&self, players: usize) -> usize {
        players - self.folded.count_ones() as usize
    }
    fn next_active(&self, from: usize, players: usize) -> usize {
        (1..=players)
            .map(|d| (from + d) % players)
            .find(|&q| self.folded >> q & 1 == 0)
            .unwrap()
    }
}

impl Game for Leduc {
    type State = LeducState;

    fn num_players(&self) -> usize {
        self.players
    }
    fn num_rounds(&self) -> usize {
        2
    }
    fn root(&self) -> LeducState {
        LeducState {
            cards: vec![],
            public: None,
            round: 0,
            hist: vec![],
            contrib: vec![1; self.players],
            round_contrib: vec![0; self.players],
            folded: 0,
            to_act: 0,
            pending: self.players,
            raises: 0,
            level: 0,
            need_public: false,
            done: false,
        }
    }
    fn turn(&self, s: &LeducState) -> Turn {
        if s.cards.len() < self.players || s.need_public {
            Turn::Chance
        } else if s.done {
            Turn::Terminal
        } else {
            Turn::Player(s.to_act)
        }
    }
    fn num_outcomes(&self, s: &LeducState) -> usize {
        if s.cards.len() < self.players {
            self.ranks * 2 - s.cards.len()
        } else if s.need_public {
            self.ranks * 2 - self.players
        } else {
            s.legal().1
        }
    }
    fn child(&self, s: &LeducState, a: usize) -> LeducState {
        let mut c = s.clone();
        let deck =
            || (0..(self.ranks * 2) as u8).filter(|x| !s.cards.contains(x) && s.public != Some(*x));
        if s.cards.len() < self.players {
            c.cards.push(deck().nth(a).unwrap());
            return c;
        }
        if s.need_public {
            c.public = deck().nth(a);
            c.need_public = false;
            c.to_act = c.next_active(self.players - 1, self.players);
            c.pending = c.active(self.players);
            return c;
        }
        let p = s.to_act;
        let act = s.legal().0[a];
        let size = if s.round == 0 { 2 } else { 4 };
        match act {
            Act::Fold => {
                c.folded |= 1 << p;
                c.pending -= 1;
                c.hist.push(b'f');
            }
            Act::Call => {
                let pay = s.level - s.round_contrib[p];
                c.contrib[p] += pay;
                c.round_contrib[p] = s.level;
                c.pending -= 1;
                c.hist.push(b'c');
            }
            Act::Raise => {
                c.level += size;
                c.contrib[p] += c.level - s.round_contrib[p];
                c.round_contrib[p] = c.level;
                c.raises += 1;
                c.pending = c.active(self.players) - 1;
                c.hist.push(b'r');
            }
        }
        if c.active(self.players) == 1 {
            c.done = true;
        } else if c.pending == 0 {
            if c.round == 0 {
                c.round = 1;
                c.need_public = true;
                c.round_contrib.fill(0);
                c.level = 0;
                c.raises = 0;
                c.hist.push(b'/');
            } else {
                c.done = true;
            }
        } else {
            c.to_act = c.next_active(p, self.players);
        }
        c
    }
    fn utilities(&self, s: &LeducState) -> Vec<f64> {
        let n = self.players;
        let pot: i32 = s.contrib.iter().sum();
        let score = |p: usize| {
            let r = (s.cards[p] / 2) as i32;
            if Some(s.cards[p] / 2) == s.public.map(|c| c / 2) {
                1000 + r
            } else {
                r
            }
        };
        let live: Vec<usize> = (0..n).filter(|&p| s.folded >> p & 1 == 0).collect();
        let best = live.iter().map(|&p| score(p)).max().unwrap();
        let winners: Vec<usize> = live.into_iter().filter(|&p| score(p) == best).collect();
        (0..n)
            .map(|p| {
                let share = if winners.contains(&p) {
                    pot as f64 / winners.len() as f64
                } else {
                    0.0
                };
                share - s.contrib[p] as f64
            })
            .collect()
    }
    fn info_key(&self, s: &LeducState) -> String {
        let p = s.to_act;
        let pubc = s.public.map_or("-".into(), |c| c.to_string());
        format!(
            "{p}|{}|{pubc}|{}",
            s.cards[p],
            String::from_utf8_lossy(&s.hist)
        )
    }
    fn in_hand(&self, s: &LeducState, p: usize) -> bool {
        s.folded >> p & 1 == 0
    }
    fn round(&self, s: &LeducState) -> usize {
        s.round as usize
    }
}
