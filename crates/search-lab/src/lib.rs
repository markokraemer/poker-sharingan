//! Pluribus-style depth-limited nested unsafe search, proven on Leduc hold'em with EXACT
//! exploitability (2p) / NashConv (3p). Brown & Sandholm, Science 2019, supplementary
//! "Further details of the real-time search algorithm" and Algorithm 2.
//!
//! Pipeline: [`blueprint`] (imperfect profiles) -> [`beliefs`] (Bayes over private cards)
//! -> [`subgame`] (chance root over the belief, betting tree of the current round, leaf
//! choice nodes over 4 continuation strategies, Linear CFR) -> [`composite`] (full-game
//! profile built round by round, evaluated with `cfr_lab::eval`).
//!
//! Search runs directly on cfr-lab's flat [`cfr_lab::Tree`]: a subgame is a *sub-forest* of
//! the real tree (so card removal and infoset identity come for free) re-packed into a new
//! `Tree` so the unmodified `Cfr` solver and exact evaluators apply.

pub mod beliefs;
pub mod blueprint;
pub mod composite;
pub mod poker;
pub mod subgame;
pub mod view;
