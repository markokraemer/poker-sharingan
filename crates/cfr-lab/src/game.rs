//! The `Game` trait: a small, explicit extensive-form game description.
//!
//! Games are state machines. [`crate::tree::Tree::build`] unrolls any `Game` into a flat
//! arena with interned information sets, so every solver and the exact best-response code
//! run unchanged on every game.

/// Whose move it is at a state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    Terminal,
    Chance,
    Player(usize),
}

pub trait Game {
    type State: Clone;

    fn num_players(&self) -> usize;
    /// Number of betting rounds (used by MCCFR-P: pruning never applies on the last round).
    fn num_rounds(&self) -> usize;
    fn root(&self) -> Self::State;
    fn turn(&self, s: &Self::State) -> Turn;
    /// Number of actions (player node) or outcomes (chance node).
    fn num_outcomes(&self, s: &Self::State) -> usize;
    /// Chance outcome probabilities; uniform by default.
    fn chance_probs(&self, s: &Self::State) -> Vec<f64> {
        let n = self.num_outcomes(s);
        vec![1.0 / n as f64; n]
    }
    /// Apply action / chance outcome `a` (index in `0..num_outcomes`).
    fn child(&self, s: &Self::State, a: usize) -> Self::State;
    /// Terminal payoffs, one per player (net of the player's own contribution).
    fn utilities(&self, s: &Self::State) -> Vec<f64>;
    /// Information-set key of the acting player; equal keys must have equal action counts.
    fn info_key(&self, s: &Self::State) -> String;
    /// False once player `p` has folded (their remaining play is irrelevant to them).
    fn in_hand(&self, s: &Self::State, p: usize) -> bool;
    /// Current betting round, `0..num_rounds`.
    fn round(&self, s: &Self::State) -> usize;
}
