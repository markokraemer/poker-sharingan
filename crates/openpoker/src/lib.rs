//! Live 6-max evaluation on Open Poker (openpoker.ai).
//!
//! Layers (bottom up):
//! - [`proto`]: tolerant parsing of server messages, builders for client messages.
//! - [`layout`]: Open Poker seats -> nlhe seats (button/blinds rotation, short-handed
//!   embedding into the 6-max game), chip scaling (x[`SCALE`]).
//! - [`sim`]: replay of the table's actions onto an `nlhe::State` plus the hero's bench
//!   `Player`; produces a distribution over concrete nlhe actions.
//! - [`mapping`]: nlhe `Action` -> legal Open Poker action (clamp/round/fallback).
//! - [`session`]: socket-free state machine (message in, messages out), unit-testable.
//! - [`live`]: websocket/REST IO, reconnect.
//! - [`trace`]: JSONL trace; [`analyze`] + [`equity`]: bb/100, CIs, all-in EV adjustment.
//!
//! Chips: Open Poker blinds are 10/20, ours 50/100, so every Open Poker chip amount is
//! multiplied by [`SCALE`] = 5 going in and rounded (and clamped to the legal range) going out.

pub mod analyze;
pub mod equity;
pub mod guard;
pub mod layout;
pub mod live;
pub mod mapping;
pub mod offline;
pub mod proto;
pub mod searchpol;
pub mod session;
pub mod sim;
pub mod trace;

/// Our chips per Open Poker chip (50/100 blinds vs 10/20).
pub const SCALE: i32 = 5;
