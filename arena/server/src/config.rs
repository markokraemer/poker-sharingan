//! Server configuration (environment variables, see README).

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// `ARENA_BIND`, default `127.0.0.1:8080`.
    pub bind: String,
    /// `ARENA_DB`, SQLite path (ignored when `ARENA_DATABASE_URL` is set) or `:memory:`, default `arena.db`.
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
    /// `ARENA_RANKED_MIN_HANDS` (default 0 = any length): a verified run sets a bot's track score only with at least this many
    /// hands, so a pile of short, wide-interval runs can't fish for a lucky score. Shorter runs still score for their owner.
    pub ranked_min_hands: u64,
    /// `ARENA_SAVE_FREE_HANDS` (default true): store hand histories of free tables too.
    pub save_free_hands: bool,
    /// `ARENA_DATABASE_URL` (`postgres://...`): use Postgres instead of SQLite.
    pub database_url: Option<String>,
    /// `ARENA_DATABASE_SCHEMA`: Postgres schema for all tables (default: the connection's `public`).
    pub database_schema: Option<String>,
    /// `ARENA_DB_POOL` (default 8): Postgres connections.
    pub db_pool: usize,
    /// `ARENA_RUNNER_TOKEN`: bearer token of the trusted eval runner (hosted attestation). Unset = no runner.
    pub runner_token: Option<String>,
    /// `ARENA_WORKER_KEYS`: `id=secret,id2=secret2` HMAC keys of match workers (`sharingan-arena run --sign-key-file`);
    /// `POST /v1/results` accepts only results signed with one of them. Unset = the endpoint refuses everything (401).
    pub worker_keys: Vec<(String, String)>,
    /// `ARENA_RATE_LIMIT_RPS` (default 50, 0 = off): sustained requests/s per bearer token (per IP when anonymous).
    pub rate_limit_rps: f64,
    /// `ARENA_RATE_LIMIT_BURST` (default 100).
    pub rate_limit_burst: f64,
    /// `ARENA_TRUST_PROXY` (default false): key anonymous requests by the first `X-Forwarded-For` hop.
    pub trust_proxy: bool,
    /// `ARENA_BOTS_PER_IP_PER_DAY` (default 0 = off; production sets 30): new bot registrations per client address per day (token bucket,
    /// so a burst of 10, then one more every 2.4 h). Stops one person from minting bots to get around per-bot quotas.
    pub bots_per_ip_per_day: f64,
    /// `ARENA_MAX_TABLES_PER_PLAYER` (default 10): open free tables a player owns or sits at.
    pub max_tables_per_player: usize,
    /// `ARENA_MAX_MATCHES_PER_PLAYER` (default 4): unfinished matches/benchmark runs a player owns.
    pub max_matches_per_player: usize,
    /// `ARENA_MAX_WS_PER_PLAYER` (default 4): simultaneous `/v1/play` sockets.
    pub max_ws_per_player: usize,
    /// `ARENA_MAX_RUN_HANDS` (default 100000): hands of one benchmark run.
    pub max_run_hands: u32,
    /// `ARENA_MAX_MATCH_HANDS` (default 1000000): `hands_planned` of one match.
    pub max_match_hands: u32,
    /// `ARENA_MAX_BODY_BYTES` (default 262144): HTTP request body.
    pub max_body_bytes: usize,
    /// `ARENA_MAX_WS_MESSAGE_BYTES` (default 65536): one client WebSocket message.
    pub max_ws_message_bytes: usize,
    /// `ARENA_SHUTDOWN_GRACE_MS` (default 20000): how long SIGTERM lets running matches finish.
    pub shutdown_grace_ms: u64,
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
            ranked_min_hands: 0,
            save_free_hands: true,
            database_url: None,
            database_schema: None,
            db_pool: 8,
            runner_token: None,
            worker_keys: Vec::new(),
            rate_limit_rps: 50.0,
            rate_limit_burst: 100.0,
            trust_proxy: false,
            bots_per_ip_per_day: 0.0,
            max_tables_per_player: 10,
            max_matches_per_player: 4,
            max_ws_per_player: 4,
            max_run_hands: 100_000,
            max_match_hands: 1_000_000,
            max_body_bytes: 256 * 1024,
            max_ws_message_bytes: 64 * 1024,
            shutdown_grace_ms: 20_000,
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
            ranked_min_hands: num("ARENA_RANKED_MIN_HANDS", d.ranked_min_hands),
            save_free_hands: flag("ARENA_SAVE_FREE_HANDS", d.save_free_hands),
            database_url: env("ARENA_DATABASE_URL"),
            database_schema: env("ARENA_DATABASE_SCHEMA"),
            db_pool: num("ARENA_DB_POOL", d.db_pool),
            runner_token: env("ARENA_RUNNER_TOKEN"),
            worker_keys: env("ARENA_WORKER_KEYS").map(|v| crate::worker::parse_keys(&v)).unwrap_or_default(),
            rate_limit_rps: num("ARENA_RATE_LIMIT_RPS", d.rate_limit_rps),
            rate_limit_burst: num("ARENA_RATE_LIMIT_BURST", d.rate_limit_burst),
            trust_proxy: flag("ARENA_TRUST_PROXY", d.trust_proxy),
            bots_per_ip_per_day: num("ARENA_BOTS_PER_IP_PER_DAY", d.bots_per_ip_per_day),
            max_tables_per_player: num("ARENA_MAX_TABLES_PER_PLAYER", d.max_tables_per_player),
            max_matches_per_player: num("ARENA_MAX_MATCHES_PER_PLAYER", d.max_matches_per_player),
            max_ws_per_player: num("ARENA_MAX_WS_PER_PLAYER", d.max_ws_per_player),
            max_run_hands: num("ARENA_MAX_RUN_HANDS", d.max_run_hands),
            max_match_hands: num("ARENA_MAX_MATCH_HANDS", d.max_match_hands),
            max_body_bytes: num("ARENA_MAX_BODY_BYTES", d.max_body_bytes),
            max_ws_message_bytes: num("ARENA_MAX_WS_MESSAGE_BYTES", d.max_ws_message_bytes),
            shutdown_grace_ms: num("ARENA_SHUTDOWN_GRACE_MS", d.shutdown_grace_ms),
        }
    }
}
