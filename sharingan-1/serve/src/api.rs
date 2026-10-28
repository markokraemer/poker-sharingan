//! Request / response types.
//!
//! Conventions follow docs/TABLE_API.md: cards are `"Ah"`, seats are table seats, chips are
//! integers, and every amount is a **"to"** (the player's total contribution on the current
//! street after the action), never a "by". Unknown request fields are ignored, so a TABLE_API
//! `your_turn` payload (whose `history` entries also carry stack/pot/...) can be forwarded as is.

use serde::{Deserialize, Serialize};

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlayerIn {
    pub seat: i64,
    /// Chips behind at the start of the hand, BEFORE blinds are posted.
    pub stack_start: i64,
    /// `false` = sitting out this hand (ignored).
    #[serde(default = "yes")]
    pub in_hand: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActionIn {
    /// Optional; when present it must match the street the engine is on.
    #[serde(default)]
    pub street: Option<String>,
    pub seat: i64,
    /// `post_sb`, `post_bb` (optional leading entries), `fold`, `check`, `call`, `bet`, `raise`, `all_in`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Raise/bet: the "to" amount (required). Call: optional, validated if present.
    #[serde(default)]
    pub to: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Options {
    /// Return the full action distribution (default true).
    #[serde(default = "yes")]
    pub include_probs: bool,
    /// Per-request hard budget in ms (capped at the server's `budget_ms`).
    #[serde(default)]
    pub time_budget_ms: Option<u64>,
    /// Seed for the action sampling (and the search); random when absent.
    #[serde(default)]
    pub seed: Option<u64>,
    /// `true` (default): sample from the strategy (the bot is a mixed strategy). `false`: most likely action.
    #[serde(default = "yes")]
    pub sample: bool,
    /// `false` = blueprint only for this request.
    #[serde(default = "yes")]
    pub search: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { include_probs: true, time_budget_ms: None, seed: None, sample: true, search: true }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActRequest {
    /// Echoed back.
    #[serde(default)]
    pub hand_id: Option<String>,
    pub small_blind: i64,
    pub big_blind: i64,
    pub button_seat: i64,
    pub hero_seat: i64,
    pub hole_cards: Vec<String>,
    /// Board in deal order (0, 3, 4 or 5 cards); must match the street the history ends on.
    #[serde(default)]
    pub board: Vec<String>,
    pub players: Vec<PlayerIn>,
    /// Every action so far, in order. Leading `post_sb`/`post_bb` entries are optional.
    #[serde(default)]
    pub history: Vec<ActionIn>,
    #[serde(default)]
    pub options: Options,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BatchRequest {
    /// Each item is an [`ActRequest`]; parsed one by one so a malformed item only fails itself.
    pub requests: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ActionOut {
    /// `fold` | `check` | `call` | `raise` (a `raise` is also a bet when nobody has bet this street).
    #[serde(rename = "type")]
    pub kind: &'static str,
    /// Total street contribution after the action (calls and raises).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<i64>,
    /// Chips moved by this action.
    pub added: i64,
    pub all_in: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProbOut {
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<i64>,
    pub p: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallLegal {
    pub to: i64,
    pub added: i64,
    pub all_in: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RaiseLegal {
    pub min_to: i64,
    pub max_to: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LegalOut {
    pub fold: bool,
    pub check: bool,
    pub call: Option<CallLegal>,
    pub raise: Option<RaiseLegal>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Timing {
    pub total_ms: f64,
    /// Wall time of the search (including a timed-out one); absent when no search ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_ms: Option<f64>,
    /// Budget this request ran under.
    pub budget_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActResponse {
    pub request_id: String,
    pub hand_id: Option<String>,
    pub model_id: String,
    pub street: &'static str,
    pub action: ActionOut,
    /// `search` | `blueprint` (no search needed / disabled) | `fallback` (search failed or timed out, or the
    /// hand could not be mapped; see `fallback_reason`).
    pub source: &'static str,
    pub fallback_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probs: Option<Vec<ProbOut>>,
    pub legal: LegalOut,
    pub timing: Timing,
    pub notes: Vec<String>,
}
