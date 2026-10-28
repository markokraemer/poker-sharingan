//! Kuhn poker with `n >= 2` players and `n + 1` cards (the OpenSpiel variant).
//!
//! Everyone antes 1 and gets one card. Players act in seat order with *pass* or *bet 1*;
//! once someone has bet, every other player (wrapping around to seats that already passed)
//! must *call* (+1) or *fold*. If nobody bets the hand goes to showdown after one orbit;
//! the highest card among non-folded players takes the pot. 2 players: game value for
//! player 1 is -1/18 and there are 12 infosets.

use crate::game::{Game, Turn};

pub struct Kuhn {
    pub players: usize,
}

impl Kuhn {
    pub fn new(players: usize) -> Self {
        assert!((2..=6).contains(&players));
        Kuhn { players }
    }
}

#[derive(Clone, Debug)]
pub struct KuhnState {
    cards: Vec<u8>,
    hist: Vec<u8>,
}

impl KuhnState {
    fn bet_idx(&self) -> Option<usize> {
        self.hist.iter().position(|&a| a == 1)
    }
}

impl Game for Kuhn {
    type State = KuhnState;

    fn num_players(&self) -> usize {
        self.players
    }
    fn num_rounds(&self) -> usize {
        1
    }
    fn root(&self) -> KuhnState {
        KuhnState {
            cards: vec![],
            hist: vec![],
        }
    }
    fn turn(&self, s: &KuhnState) -> Turn {
        let n = self.players;
        if s.cards.len() < n {
            return Turn::Chance;
        }
        let done = match s.bet_idx() {
            None => s.hist.len() == n,
            Some(b) => s.hist.len() >= b + n,
        };
        if done {
            Turn::Terminal
        } else {
            Turn::Player(s.hist.len() % n)
        }
    }
    fn num_outcomes(&self, s: &KuhnState) -> usize {
        if s.cards.len() < self.players {
            self.players + 1 - s.cards.len()
        } else {
            2
        }
    }
    fn child(&self, s: &KuhnState, a: usize) -> KuhnState {
        let mut c = s.clone();
        if s.cards.len() < self.players {
            let card = (0..=self.players as u8)
                .filter(|x| !s.cards.contains(x))
                .nth(a)
                .unwrap();
            c.cards.push(card);
        } else {
            c.hist.push(a as u8);
        }
        c
    }
    fn utilities(&self, s: &KuhnState) -> Vec<f64> {
        let n = self.players;
        let mut contrib = vec![1.0; n];
        let mut folded = vec![false; n];
        let b = s.bet_idx();
        for (i, &a) in s.hist.iter().enumerate() {
            contrib[i % n] += a as f64;
            if a == 0 && b.is_some_and(|b| i > b) {
                folded[i % n] = true;
            }
        }
        let pot: f64 = contrib.iter().sum();
        let winner = (0..n)
            .filter(|&p| !folded[p])
            .max_by_key(|&p| s.cards[p])
            .unwrap();
        (0..n)
            .map(|p| {
                if p == winner {
                    pot - contrib[p]
                } else {
                    -contrib[p]
                }
            })
            .collect()
    }
    fn info_key(&self, s: &KuhnState) -> String {
        let p = s.hist.len() % self.players;
        let h: String = s
            .hist
            .iter()
            .map(|&a| if a == 1 { 'b' } else { 'p' })
            .collect();
        format!("{p}:{}:{h}", s.cards[p])
    }
    fn in_hand(&self, s: &KuhnState, p: usize) -> bool {
        let n = self.players;
        match s.bet_idx() {
            Some(b) => !s
                .hist
                .iter()
                .enumerate()
                .any(|(i, &a)| i > b && a == 0 && i % n == p),
            None => true,
        }
    }
    fn round(&self, _: &KuhnState) -> usize {
        0
    }
}
