//! Small-game CFR laboratory: games with computable ground truth, a family of regret
//! minimisers, and exact best-response exploitability. Purpose: prove the regret code is
//! correct before it is scaled to six-player NLHE.

pub mod cfr;
pub mod eval;
pub mod game;
pub mod kuhn;
pub mod leduc;
pub mod mccfr;
pub mod tree;

pub use game::{Game, Turn};
pub use tree::{Profile, Tree};

/// Common interface of every solver in this crate.
pub trait Solver {
    /// One full iteration (all players updated).
    fn iterate(&mut self);
    fn iterations(&self) -> u64;
    /// Average strategy profile (the one with convergence guarantees in 2p zero-sum).
    fn average(&self) -> Profile;
    /// Current iterate (regret matching on current regrets).
    fn current(&self) -> Profile;
}

pub(crate) const MAX_ACTIONS: usize = 16;
