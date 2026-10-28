//! Referee: measures how close a bench policy plays to GTO-solver references, spot by spot.
//!
//! * [`pokerbench`]: PokerBench (GTO Wizard-labelled 6-max 100bb spots): parser, engine
//!   replay, action-accuracy and probability-mass scoring with breakdowns.
//! * [`solver`]: exact heads-up river solver (vector CFR+, 1326 hands, card removal) with
//!   best-response EV-loss computation.
//! * [`river`]: river referee: Bayes ranges from the policy, equilibrium of the spot, EV loss
//!   in mbb of the policy's river strategy.
//! * [`replay`]: checked replay of external histories on the `nlhe` engine.
//! * [`policies`]: policy specs for the CLI (`builtin:tag`, `blueprint:<path>`).
//!
//! Seeding: every random stream is derived per item with [`bench::mix`], never per thread.

pub mod pokerbench;
pub mod policies;
pub mod replay;
pub mod river;
pub mod solver;
pub mod toy;
