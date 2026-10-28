//! Imperfect blueprints for Leduc.
//!
//! * [`mccfr_blueprint`]: under-trained external-sampling MCCFR-P (Pluribus Alg. 1) average.
//! * [`abstract_blueprint`]: card-abstracted blueprint. Round-2 information is coarsened to
//!   {paired with board, unpaired high card, unpaired low card} (suits and exact ranks
//!   dropped, own round-1 card forgotten = imperfect recall, as in Pluribus' bucketing);
//!   round 1 is exact. The abstract game is solved with CFR+ and lifted to the real game by
//!   mapping each real infoset to its abstract key. Search later re-solves round 2 with the
//!   lossless information, mirroring "lossless current round" in the supplementary.
//! * [`equilibrium_blueprint`]: CFR+ on the real game (the "exact blueprint" control).

use cfr_lab::cfr::{Cfr, Variant};
use cfr_lab::leduc::Leduc;
use cfr_lab::mccfr::{Mccfr, MccfrConfig};
use cfr_lab::{Game, Profile, Solver, Tree, Turn};
use std::collections::HashMap;

pub fn mccfr_blueprint(tree: &Tree, iters: u64, seed: u64) -> Profile {
    let mut s = Mccfr::new(tree, MccfrConfig::small_game(iters, seed));
    for _ in 0..iters {
        s.iterate();
    }
    s.average()
}

pub fn equilibrium_blueprint(tree: &Tree, iters: u64) -> Profile {
    let mut s = Cfr::new(tree, Variant::Plus);
    for _ in 0..iters {
        s.iterate();
    }
    s.average()
}

/// Abstract infoset key from a Leduc key `"p|card|board|hist"`. Round 1 (board "-") is kept.
pub fn abstract_key(key: &str, ranks: usize) -> String {
    let f: Vec<&str> = key.splitn(4, '|').collect();
    if f[2] == "-" {
        return key.to_string();
    }
    let (card, board): (usize, usize) = (f[1].parse().unwrap(), f[2].parse().unwrap());
    let class = if card / 2 == board / 2 {
        'P'
    } else if card / 2 == ranks - 1 {
        'H'
    } else {
        'L'
    };
    format!("{}|{class}|{}", f[0], f[3])
}

/// Leduc with round-2 infosets coarsened by [`abstract_key`].
pub struct AbstractLeduc<'a>(pub &'a Leduc);

impl Game for AbstractLeduc<'_> {
    type State = <Leduc as Game>::State;
    fn num_players(&self) -> usize {
        self.0.num_players()
    }
    fn num_rounds(&self) -> usize {
        self.0.num_rounds()
    }
    fn root(&self) -> Self::State {
        self.0.root()
    }
    fn turn(&self, s: &Self::State) -> Turn {
        self.0.turn(s)
    }
    fn num_outcomes(&self, s: &Self::State) -> usize {
        self.0.num_outcomes(s)
    }
    fn chance_probs(&self, s: &Self::State) -> Vec<f64> {
        self.0.chance_probs(s)
    }
    fn child(&self, s: &Self::State, a: usize) -> Self::State {
        self.0.child(s, a)
    }
    fn utilities(&self, s: &Self::State) -> Vec<f64> {
        self.0.utilities(s)
    }
    fn info_key(&self, s: &Self::State) -> String {
        abstract_key(&self.0.info_key(s), self.0.ranks)
    }
    fn in_hand(&self, s: &Self::State, p: usize) -> bool {
        self.0.in_hand(s, p)
    }
    fn round(&self, s: &Self::State) -> usize {
        self.0.round(s)
    }
}

/// Solve the abstract game with CFR+ and lift the average strategy to `real`.
pub fn abstract_blueprint(game: &Leduc, real: &Tree, iters: u64) -> Profile {
    let abs = Tree::build(&AbstractLeduc(game));
    let avg = equilibrium_blueprint(&abs, iters);
    let idx: HashMap<(usize, &str), usize> = abs
        .info
        .iter()
        .enumerate()
        .map(|(i, x)| ((x.player, x.key.as_str()), i))
        .collect();
    let mut out = vec![0.0; real.total_actions];
    for i in &real.info {
        let a = &abs.info[idx[&(i.player, abstract_key(&i.key, game.ranks).as_str())]];
        out[i.offset..i.offset + i.n_actions]
            .copy_from_slice(&avg[a.offset..a.offset + a.n_actions]);
    }
    out
}

/// Infosets of rounds `< round` from `early`, the rest from `late` (a blueprint whose only
/// flaw is its early-round play: isolates what depth-limited leaf values can repair).
pub fn splice(tree: &Tree, early: &[f64], late: &[f64], round: usize) -> Profile {
    let mut out = late.to_vec();
    for i in tree.info.iter().filter(|i| i.round < round) {
        out[i.offset..i.offset + i.n_actions]
            .copy_from_slice(&early[i.offset..i.offset + i.n_actions]);
    }
    out
}
