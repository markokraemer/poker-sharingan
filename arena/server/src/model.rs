//! Wire and storage types (TABLE_API.md sections 2, 5, 8, 9). Chips are integers.

use crate::error::{ApiError, Res};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn d_variant() -> String {
    "nlhe".into()
}
fn d_seats() -> u8 {
    6
}
fn d_sb() -> i64 {
    50
}
fn d_bb() -> i64 {
    100
}
fn d_stack() -> i64 {
    10_000
}
fn d_time() -> u64 {
    5000
}
fn d_timeout_action() -> String {
    "check_fold".into()
}
fn d_min_players() -> u8 {
    6
}
fn d_odd() -> String {
    "first_left_of_button".into()
}
fn d_show() -> String {
    "standard".into()
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StackMode {
    #[default]
    Reset,
    Carry,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Rake {
    #[serde(default)]
    pub percent: u32,
    #[serde(default)]
    pub cap: u32,
}

/// Scripted dealing for `mode:"sandbox"` tables (conformance vectors).
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Sandbox {
    /// Deck prefixes, one per hand in order (e.g. `["Ah","Kd",...]`); the rest of each deck follows the seeded order.
    #[serde(default)]
    pub decks: Vec<Vec<String>>,
    /// Fixed button seat for every hand.
    #[serde(default)]
    pub button_seat: Option<usize>,
    /// Starting stacks per seat for the first hand only (stack_mode reset otherwise applies).
    #[serde(default)]
    pub stacks: Option<Vec<i64>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TableConfig {
    #[serde(default = "d_variant")]
    pub variant: String,
    #[serde(default = "d_seats")]
    pub max_seats: u8,
    #[serde(default = "d_sb")]
    pub small_blind: i64,
    #[serde(default = "d_bb")]
    pub big_blind: i64,
    #[serde(default)]
    pub ante: i64,
    #[serde(default)]
    pub straddle: bool,
    #[serde(default = "d_stack")]
    pub starting_stack: i64,
    #[serde(default)]
    pub stack_mode: StackMode,
    #[serde(default = "d_time")]
    pub decision_time_ms: u64,
    #[serde(default)]
    pub time_bank_ms: u64,
    #[serde(default = "d_timeout_action")]
    pub timeout_action: String,
    #[serde(default)]
    pub rake: Rake,
    #[serde(default = "d_min_players")]
    pub min_players_to_deal: u8,
    #[serde(default = "d_odd")]
    pub odd_chip_rule: String,
    #[serde(default = "d_show")]
    pub show_mode: String,
    /// `"sandbox"` enables scripted decks; omitted/`"normal"` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<Sandbox>,
    /// Table seed (free tables); matches use the match seed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Pause between hands on free tables (default from server config).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hand_delay_ms: Option<u64>,
}

impl Default for TableConfig {
    fn default() -> Self {
        serde_json::from_value(serde_json::json!({})).unwrap()
    }
}

impl TableConfig {
    pub fn validate(&self) -> Res<()> {
        let bad = |m: &str| Err(ApiError::bad_request(m.to_string()));
        if self.variant != "nlhe" {
            return bad("variant must be nlhe");
        }
        if !(2..=6).contains(&self.max_seats) {
            return bad("max_seats must be 2..6");
        }
        if self.small_blind <= 0 || self.big_blind < self.small_blind {
            return bad("blinds must satisfy 0 < small_blind <= big_blind");
        }
        if self.ante != 0 || self.straddle {
            return bad("ante and straddle are not supported (ante=0, straddle=false)");
        }
        if self.starting_stack < self.big_blind || self.starting_stack > 1_000_000_000 {
            return bad("starting_stack must be in [big_blind, 1e9]");
        }
        if self.rake.percent != 0 || self.rake.cap != 0 {
            return bad("rake is not supported");
        }
        if self.timeout_action != "check_fold" {
            return bad("timeout_action must be check_fold");
        }
        if self.decision_time_ms == 0 || self.decision_time_ms > 3_600_000 {
            return bad("decision_time_ms must be in 1..3600000");
        }
        if self.min_players_to_deal < 2 || self.min_players_to_deal > self.max_seats {
            return bad("min_players_to_deal must be in 2..max_seats");
        }
        if let Some(m) = &self.mode {
            if m != "sandbox" && m != "ranked" && m != "benchmark" && m != "normal" {
                return bad("mode must be sandbox, ranked or benchmark");
            }
        }
        if self.sandbox.is_some() && !self.is_sandbox() {
            return bad("sandbox options require mode=sandbox");
        }
        Ok(())
    }
    pub fn is_benchmark(&self) -> bool {
        self.mode.as_deref() == Some("benchmark")
    }
    pub fn is_sandbox(&self) -> bool {
        self.mode.as_deref() == Some("sandbox")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DuplicateConfig {
    pub enabled: bool,
    #[serde(default = "d_rot")]
    pub rotations: u8,
    #[serde(default = "d_deck_from")]
    pub deck_from: String,
}
fn d_rot() -> u8 {
    6
}
fn d_deck_from() -> String {
    "seed".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlayerRecord {
    pub player_id: String,
    pub name: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub house: bool,
    #[serde(skip)]
    pub token_hash: String,
    #[serde(skip)]
    pub webhook_url: Option<String>,
    #[serde(skip)]
    pub webhook_secret: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MatchRecord {
    pub match_id: String,
    pub table_id: String,
    pub participants: Vec<String>,
    pub hands_planned: u32,
    pub duplicate: Option<DuplicateConfig>,
    pub seed: String,
    pub status: String,
    pub hands_played: u32,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    pub table_config: TableConfig,
    /// "full" | "cards" | "off"
    pub aivat: String,
    /// One table per lane; lanes play disjoint blocks of deals in parallel.
    #[serde(default)]
    pub table_ids: Vec<String>,
    #[serde(default)]
    pub lanes: u32,
    /// Set for benchmark-track runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

/// A benchmark-track run of one bot against a frozen panel.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunRecord {
    pub run_id: String,
    pub track: String,
    pub panel_version: String,
    pub panel: Vec<String>,
    pub player_id: String,
    pub name: String,
    pub match_id: String,
    pub seed: String,
    pub hands_planned: u32,
    pub hands_done: u32,
    /// "running" | "finished" | "aborted"
    pub status: String,
    /// True only when the server itself ran the bot (house/hosted bots); remote bots are unverified.
    pub verified: bool,
    pub mbb_per_game: Option<f64>,
    pub ci95_mbb: Option<f64>,
    pub method: Option<String>,
    pub blueprint_fingerprint: Option<String>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct VsCell {
    pub mbb: f64,
    pub ci95_mbb: f64,
    pub hands: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct TrackCell {
    pub run_id: String,
    pub mbb_per_game: f64,
    pub ci95_mbb: f64,
    pub hands: u32,
    pub verified: bool,
    pub method: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionEvent {
    pub hand_id: String,
    pub seq: u32,
    pub action_id: String,
    pub street: String,
    pub seat: u8,
    #[serde(rename = "type")]
    pub kind: String,
    pub to: i64,
    pub added: i64,
    pub all_in: bool,
    pub forced: bool,
    pub timeout: bool,
    pub stack: i64,
    pub pot: i64,
    pub current_bet: i64,
    pub min_raise_to: i64,
    pub next_to_act: Option<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PotResult {
    pub amount: i64,
    pub eligible_seats: Vec<u8>,
    pub winners: Vec<u8>,
    pub share: Vec<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResultRow {
    pub seat: u8,
    pub player_id: String,
    pub net: i64,
    pub stack_end: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistPlayer {
    pub seat: u8,
    pub player_id: String,
    pub name: String,
    pub kind: String,
    pub stack_start: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistConfig {
    pub small_blind: i64,
    pub big_blind: i64,
    pub ante: i64,
    pub starting_stack: i64,
    pub stack_mode: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HandHistory {
    pub hand_id: String,
    pub hand_no: u32,
    pub match_id: Option<String>,
    pub table_id: String,
    pub deal_no: Option<u32>,
    pub rotation: Option<u32>,
    pub started_at: String,
    pub ended_at: String,
    pub config: HistConfig,
    pub button_seat: u8,
    pub sb_seat: u8,
    pub bb_seat: u8,
    pub players: Vec<HistPlayer>,
    pub hole_cards: BTreeMap<u8, [String; 2]>,
    pub board: Vec<String>,
    pub runout: Vec<String>,
    pub deck: Vec<String>,
    pub deck_salt: String,
    pub actions: Vec<ActionEvent>,
    pub decision_times_ms: BTreeMap<String, u64>,
    pub pots: Vec<PotResult>,
    pub results: Vec<ResultRow>,
    pub ended_by: String,
    pub anomalies: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Standing {
    pub player_id: String,
    /// Participant slot (a house bot may fill several).
    #[serde(default)]
    pub slot: u8,
    pub name: String,
    pub hands: u64,
    pub net_chips: i64,
    pub bb_per_100: f64,
    pub mbb_per_hand: f64,
    pub ci95_bb_per_100: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplicate_bb_per_100: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplicate_mbb_per_hand: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplicate_ci95_mbb_per_hand: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aivat_mbb_per_hand: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aivat_ci95_mbb_per_hand: Option<f64>,
    /// How `score_mbb_per_hand` was produced: "aivat" | "duplicate" | "raw".
    pub method: String,
    pub score_mbb_per_hand: f64,
    pub score_ci95_mbb_per_hand: f64,
    /// Same as `score_mbb_per_hand` / `score_ci95_mbb_per_hand` (SDK-facing names).
    pub mbb_per_game: f64,
    pub ci95_mbb: f64,
    pub deals: u64,
}

/// One stored per-match, per-player result (feeds the leaderboard).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredResult {
    pub match_id: String,
    pub player_id: String,
    pub hands: u64,
    pub deals: u64,
    pub net_chips: i64,
    pub score_mbb: f64,
    pub se_mbb: f64,
    pub method: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct LeaderboardRow {
    pub rank: usize,
    pub player_id: String,
    pub name: String,
    pub house: bool,
    pub matches: u64,
    pub hands: u64,
    /// General (all matches) luck-corrected score; null if the bot has not reached the hands threshold.
    pub score_mbb_per_hand: Option<f64>,
    pub ci95_mbb_per_hand: Option<f64>,
    pub score_bb_per_100: Option<f64>,
    pub methods: Vec<String>,
    /// Blended mbb/game vs the `ladder-v1` panel (latest finished run), if any.
    pub vs_ladder_v1: Option<VsCell>,
    /// Same for the `sharingan-1` panel.
    pub vs_sharingan_1: Option<VsCell>,
    /// True if the displayed track results were run by the server itself (house/hosted bots).
    pub verified: bool,
    /// One cell per benchmark track (latest finished run).
    pub tracks: std::collections::BTreeMap<String, TrackCell>,
}
