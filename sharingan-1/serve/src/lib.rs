//! SHARINGAN-1 inference API.
//!
//! Stateless: every request carries the full public hand history, the server replays it through
//! `engine/nlhe` (validating legality), asks the bot (compact blueprint + turn/river search under
//! a hard time budget) and returns a legal action.
//!
//! * [`api`]: request/response JSON (shapes and "to" semantics of docs/TABLE_API.md).
//! * [`replay`]: history -> engine state (real chips) + scaled engine/player (50/100 blinds).
//! * [`decide`]: [`decide::Core`], the shared bot (blueprint loaded once, search worker gate).
//! * [`server`]: axum router, Bearer auth, per-key rate limit, JSON logs.
//! * [`config`]: env/flag configuration.

pub mod api;
pub mod config;
pub mod decide;
pub mod error;
pub mod log;
pub mod replay;
pub mod server;
