//! Server configuration (environment variables, see README).

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// `ARENA_BIND`, default `127.0.0.1:8080`.
    pub bind: String,
    /// `ARENA_DB`, SQLite path or `:memory:`, default `arena.db`.
    pub db_path: String,
    /// `ARENA_ADMIN_TOKEN`; if unset a random one is generated and logged at startup.
    pub admin_token: Option<String>,
    /// `ARENA_BLUEPRINT`: path to a compact blueprint (.plcp) to seat as house bot `blueprint`.
    pub blueprint: Option<PathBuf>,
    /// `ARENA_BUCKETS`: bucketer spec, e.g. `tables:data/buckets-200` (default `coarse`).
    pub buckets: String,
    /// `ARENA_SIZING`: bet-size menu of the blueprint (default `medium`).
    pub sizing: String,
    /// `ARENA_OPEN_REGISTRATION` (default true): anyone may `POST /v1/bots`.
    pub open_registration: bool,
    /// `ARENA_ENABLE_WEBHOOKS` (default false): allow bots with a `webhook_url` (server-side requests).
    pub enable_webhooks: bool,
    /// `ARENA_HAND_DELAY_MS` (default 300): pause between hands on free tables.
    pub hand_delay_ms: u64,
    /// `ARENA_RECONNECT_GRACE_MS` (default 2000): how long a turn waits for a disconnected remote seat.
    pub reconnect_grace_ms: u64,
    /// `ARENA_LEADERBOARD_MIN_HANDS` (default 1000).
    pub min_leaderboard_hands: u64,
    /// `ARENA_SAVE_FREE_HANDS` (default true): store hand histories of free tables too.
    pub save_free_hands: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            bind: "127.0.0.1:8080".into(),
            db_path: "arena.db".into(),
            admin_token: None,
            blueprint: None,
            buckets: "coarse".into(),
            sizing: "medium".into(),
            open_registration: true,
            enable_webhooks: false,
            hand_delay_ms: 300,
            reconnect_grace_ms: 2000,
            min_leaderboard_hands: 1000,
            save_free_hands: true,
        }
    }
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

fn flag(k: &str, d: bool) -> bool {
    env(k).map(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on")).unwrap_or(d)
}

fn num<T: std::str::FromStr>(k: &str, d: T) -> T {
    env(k).and_then(|v| v.parse().ok()).unwrap_or(d)
}

impl ServerConfig {
    pub fn from_env() -> Self {
        let d = ServerConfig::default();
        ServerConfig {
            bind: env("ARENA_BIND").unwrap_or(d.bind),
            db_path: env("ARENA_DB").unwrap_or(d.db_path),
            admin_token: env("ARENA_ADMIN_TOKEN"),
            blueprint: env("ARENA_BLUEPRINT").map(PathBuf::from),
            buckets: env("ARENA_BUCKETS").unwrap_or(d.buckets),
            sizing: env("ARENA_SIZING").unwrap_or(d.sizing),
            open_registration: flag("ARENA_OPEN_REGISTRATION", d.open_registration),
            enable_webhooks: flag("ARENA_ENABLE_WEBHOOKS", d.enable_webhooks),
            hand_delay_ms: num("ARENA_HAND_DELAY_MS", d.hand_delay_ms),
            reconnect_grace_ms: num("ARENA_RECONNECT_GRACE_MS", d.reconnect_grace_ms),
            min_leaderboard_hands: num("ARENA_LEADERBOARD_MIN_HANDS", d.min_leaderboard_hands),
            save_free_hands: flag("ARENA_SAVE_FREE_HANDS", d.save_free_hands),
        }
    }
}
