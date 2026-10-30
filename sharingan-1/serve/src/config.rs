//! Configuration from environment variables and `--flags` (flags win).
//!
//! Defaults mirror the production live bot (arena/openpoker): search on the turn and river only
//! (`--set flop=0`), 5 s hard budget, 16 search threads, `--max-stack-bb 150`.
//!
//! | env | flag | default |
//! |---|---|---|
//! | `SHARINGAN_BLUEPRINT` | `--blueprint` | required (compact PLCP1 file) |
//! | `SHARINGAN_BUCKETS` | `--buckets` | `data/buckets-200` (`coarse` for toy blueprints) |
//! | `SHARINGAN_SIZING` | `--sizing` | `medium` (`tiny` \| `local` \| `medium` \| `pluribus`) |
//! | `SHARINGAN_SET` | `--set` | `flop=0` (`k=v,...`, same keys as `openpoker --set`) |
//! | `SHARINGAN_SEARCH_BUCKETS` | `--search-buckets` | off |
//! | `SHARINGAN_NO_SEARCH` | `--no-search` | off (blueprint only) |
//! | `SHARINGAN_BUDGET_MS` | `--budget-ms` | 5000 (hard per-request search budget) |
//! | `SHARINGAN_SEARCH_THREADS` | `--search-threads` | 16 |
//! | `SHARINGAN_SEARCH_CONCURRENCY` | `--search-concurrency` | 2 (searches running at once) |
//! | `SHARINGAN_QUEUE_MS` | `--queue-ms` | 2000 (max wait for a search slot, then blueprint) |
//! | `SHARINGAN_MAX_STACK_BB` | `--max-stack-bb` | 150 (0 = off) |
//! | `SHARINGAN_BIND` | `--bind` | `0.0.0.0:8080` |
//! | `SHARINGAN_API_KEYS` | `--api-keys` | required unless `--no-auth`: `name=key,name2=key2` or bare keys |
//! | `SHARINGAN_API_KEYS_FILE` | `--api-keys-file` | one `name=key` or `key` per line |
//! | `SHARINGAN_NO_AUTH` | `--no-auth` | off |
//! | `SHARINGAN_RATE_RPS` | `--rate-rps` | 200 (refill rate per key, requests/s) |
//! | `SHARINGAN_RATE_BURST` | `--rate-burst` | 400 |
//! | `SHARINGAN_RATE_LIMITS` | `--rate-limits` | none: per-key overrides `name=rps[:burst],...` (burst defaults to 2 x rps) |
//! | `SHARINGAN_ARENA_AUTH_URL` | `--arena-auth-url` | off: also accept arena player tokens (`sk_...`), checked once via `GET {url}/v1/me` |
//! | `SHARINGAN_PUBLIC_RPS` | `--public-rps` | 10 (refill rate per arena player) |
//! | `SHARINGAN_PUBLIC_BURST` | `--public-burst` | 20 |
//! | `SHARINGAN_MAX_BATCH` | `--max-batch` | 64 |
//! | `SHARINGAN_MAX_INFLIGHT` | `--max-inflight` | 256 (concurrent decisions; more -> 503) |
//! | `SHARINGAN_MODEL_ID` | `--model-id` | `sharingan-1` |
//! | `SHARINGAN_WARM` | `--warm` | off (read the whole blueprint ahead; only if it fits in RAM) |
//! | `SHARINGAN_LOG` | | `on` (`off` silences JSON logs) |

use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub blueprint: PathBuf,
    pub buckets: String,
    pub sizing: String,
    pub sets: String,
    pub search_buckets: Option<String>,
    pub search: bool,
    pub budget_ms: u64,
    pub search_threads: usize,
    pub search_concurrency: usize,
    pub queue_ms: u64,
    pub max_stack_bb: f64,
    pub bind: String,
    /// `(name, key)`; empty with `auth == false` = open server.
    pub api_keys: Vec<(String, String)>,
    pub auth: bool,
    pub rate_rps: f64,
    pub rate_burst: f64,
    /// Per-key overrides (by key name) of `(rps, burst)`.
    pub rate_overrides: HashMap<String, (f64, f64)>,
    /// Arena base URL whose `GET /v1/me` vouches for player tokens (public access); `None` = API keys only.
    pub arena_auth_url: Option<String>,
    pub public_rps: f64,
    pub public_burst: f64,
    pub max_batch: usize,
    pub max_inflight: usize,
    pub model_id: String,
    pub warm: bool,
    /// Test hook: sleep this long inside the search thread (to force a timeout). Env `SHARINGAN_DEBUG_SEARCH_DELAY_MS`.
    pub debug_search_delay_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            blueprint: PathBuf::new(),
            buckets: "data/buckets-200".into(),
            sizing: "medium".into(),
            sets: "flop=0".into(),
            search_buckets: None,
            search: true,
            budget_ms: 5000,
            search_threads: 16,
            search_concurrency: 2,
            queue_ms: 2000,
            max_stack_bb: 150.0,
            bind: "0.0.0.0:8080".into(),
            api_keys: vec![],
            auth: true,
            rate_rps: 200.0,
            rate_burst: 400.0,
            rate_overrides: HashMap::new(),
            arena_auth_url: None,
            public_rps: 10.0,
            public_burst: 20.0,
            max_batch: 64,
            max_inflight: 256,
            model_id: "sharingan-1".into(),
            warm: false,
            debug_search_delay_ms: 0,
        }
    }
}

pub const USAGE: &str = "usage: sharingan-serve --blueprint FILE.plcp [--buckets DIR|coarse] [--sizing medium] [--set flop=0] \
[--budget-ms 5000] [--search-threads 16] [--search-concurrency 2] [--max-stack-bb 150] [--bind 0.0.0.0:8080] \
[--api-keys name=key,...] [--api-keys-file F] [--no-auth] [--no-search] [--rate-rps 200] [--rate-burst 400] [--rate-limits name=rps[:burst],...] \
[--arena-auth-url URL] [--public-rps 10] [--public-burst 20] [--model-id ID]\n\
(every flag also reads SHARINGAN_<FLAG> from the environment; see sharingan-1/serve/README.md)";

/// `name=rps[:burst],...` (burst defaults to 2 x rps).
fn parse_rate_limits(s: &str) -> Result<HashMap<String, (f64, f64)>, String> {
    let mut out = HashMap::new();
    for tok in s.split([',', '\n']).map(str::trim).filter(|t| !t.is_empty()) {
        let bad = || format!("--rate-limits: bad entry {tok:?} (want name=rps[:burst])");
        let (name, v) = tok.split_once('=').ok_or_else(bad)?;
        let (rps, burst) = match v.split_once(':') {
            Some((r, b)) => (r.trim().parse::<f64>().map_err(|_| bad())?, b.trim().parse::<f64>().map_err(|_| bad())?),
            None => {
                let r = v.trim().parse::<f64>().map_err(|_| bad())?;
                (r, (2.0 * r).max(1.0))
            }
        };
        if !(rps > 0.0) || !(burst >= 1.0) || name.trim().is_empty() {
            return Err(bad());
        }
        out.insert(name.trim().to_string(), (rps, burst));
    }
    Ok(out)
}

fn parse_keys(s: &str, out: &mut Vec<(String, String)>) {
    for tok in s.split([',', '\n']).map(str::trim).filter(|t| !t.is_empty() && !t.starts_with('#')) {
        let (name, key) = match tok.split_once('=') {
            Some((n, k)) => (n.trim().to_string(), k.trim().to_string()),
            None => (format!("key{}", out.len()), tok.to_string()),
        };
        if !key.is_empty() {
            out.push((name, key));
        }
    }
}

impl Config {
    /// `env` looks up an environment variable; `args` are the command-line arguments after argv[0].
    pub fn parse(env: &dyn Fn(&str) -> Option<String>, args: &[String]) -> Result<Config, String> {
        let mut kv: HashMap<String, String> = HashMap::new();
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            let Some(k) = a.strip_prefix("--") else { return Err(format!("unexpected argument {a}\n{USAGE}")) };
            if let Some((k, v)) = k.split_once('=') {
                kv.insert(k.to_string(), v.to_string());
                i += 1;
            } else if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                kv.insert(k.to_string(), args[i + 1].clone());
                i += 2;
            } else {
                kv.insert(k.to_string(), "true".into());
                i += 1;
            }
        }
        const KNOWN: &[&str] = &[
            "blueprint", "buckets", "sizing", "set", "search-buckets", "no-search", "budget-ms", "search-threads",
            "search-concurrency", "queue-ms", "max-stack-bb", "bind", "api-keys", "api-keys-file", "no-auth", "rate-rps",
            "rate-burst", "rate-limits", "arena-auth-url", "public-rps", "public-burst", "max-batch", "max-inflight", "model-id", "warm", "help",
        ];
        for k in kv.keys() {
            if !KNOWN.contains(&k.as_str()) {
                return Err(format!("unknown flag --{k}\n{USAGE}"));
            }
        }
        if kv.contains_key("help") {
            return Err(USAGE.to_string());
        }
        // flag > env
        let get = |k: &str| -> Option<String> {
            kv.get(k).cloned().or_else(|| env(&format!("SHARINGAN_{}", k.to_uppercase().replace('-', "_")))).filter(|v| !v.is_empty())
        };
        let flag = |k: &str| get(k).is_some_and(|v| !matches!(v.as_str(), "0" | "false" | "off" | "no"));
        fn num<T: std::str::FromStr>(k: &str, v: Option<String>, d: T) -> Result<T, String> {
            match v {
                None => Ok(d),
                Some(s) => s.parse().map_err(|_| format!("--{k}: cannot parse {s:?}")),
            }
        }
        let d = Config::default();
        let blueprint = get("blueprint").map(PathBuf::from).ok_or_else(|| format!("--blueprint (or SHARINGAN_BLUEPRINT) is required\n{USAGE}"))?;
        let auth = !flag("no-auth");
        let mut api_keys = vec![];
        if let Some(s) = get("api-keys") {
            parse_keys(&s, &mut api_keys);
        }
        if let Some(f) = get("api-keys-file") {
            let text = std::fs::read_to_string(&f).map_err(|e| format!("--api-keys-file {f}: {e}"))?;
            parse_keys(&text, &mut api_keys);
        }
        let arena_auth_url = get("arena-auth-url").map(|u| u.trim_end_matches('/').to_string());
        if auth && api_keys.is_empty() && arena_auth_url.is_none() {
            return Err("API keys required: set SHARINGAN_API_KEYS / --api-keys / --api-keys-file or --arena-auth-url, or pass --no-auth for an open server".into());
        }
        let cfg = Config {
            blueprint,
            buckets: get("buckets").unwrap_or(d.buckets),
            sizing: get("sizing").unwrap_or(d.sizing),
            sets: get("set").unwrap_or(d.sets),
            search_buckets: get("search-buckets"),
            search: !flag("no-search"),
            budget_ms: num("budget-ms", get("budget-ms"), d.budget_ms)?,
            search_threads: num("search-threads", get("search-threads"), d.search_threads)?,
            search_concurrency: num("search-concurrency", get("search-concurrency"), d.search_concurrency)?,
            queue_ms: num("queue-ms", get("queue-ms"), d.queue_ms)?,
            max_stack_bb: num("max-stack-bb", get("max-stack-bb"), d.max_stack_bb)?,
            bind: get("bind").unwrap_or(d.bind),
            api_keys,
            auth,
            rate_rps: num("rate-rps", get("rate-rps"), d.rate_rps)?,
            rate_burst: num("rate-burst", get("rate-burst"), d.rate_burst)?,
            rate_overrides: get("rate-limits").map(|s| parse_rate_limits(&s)).transpose()?.unwrap_or_default(),
            arena_auth_url,
            public_rps: num("public-rps", get("public-rps"), d.public_rps)?,
            public_burst: num("public-burst", get("public-burst"), d.public_burst)?,
            max_batch: num("max-batch", get("max-batch"), d.max_batch)?,
            max_inflight: num("max-inflight", get("max-inflight"), d.max_inflight)?,
            model_id: get("model-id").unwrap_or(d.model_id),
            warm: flag("warm"),
            debug_search_delay_ms: num("debug-search-delay-ms", env("SHARINGAN_DEBUG_SEARCH_DELAY_MS"), 0)?,
        };
        if cfg.budget_ms == 0 {
            return Err("--budget-ms must be > 0".into());
        }
        if cfg.search_concurrency == 0 || cfg.search_threads == 0 || cfg.max_batch == 0 || cfg.max_inflight == 0 {
            return Err("--search-concurrency, --search-threads, --max-batch and --max-inflight must be > 0".into());
        }
        if !(cfg.max_stack_bb >= 0.0) || !(cfg.rate_rps > 0.0) || !(cfg.rate_burst >= 1.0) || !(cfg.public_rps > 0.0) || !(cfg.public_burst >= 1.0) {
            return Err("--max-stack-bb must be >= 0, --rate-rps / --public-rps > 0, --rate-burst / --public-burst >= 1".into());
        }
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(env: &[(&str, &str)], args: &[&str]) -> Result<Config, String> {
        let m: HashMap<String, String> = env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let a: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        Config::parse(&|k| m.get(k).cloned(), &a)
    }

    #[test]
    fn defaults_mirror_live_bot() {
        let c = p(&[("SHARINGAN_BLUEPRINT", "/x.plcp"), ("SHARINGAN_API_KEYS", "alice=k1,k2")], &[]).unwrap();
        assert_eq!(c.sets, "flop=0");
        assert_eq!(c.max_stack_bb, 150.0);
        assert_eq!(c.budget_ms, 5000);
        assert_eq!((c.rate_rps, c.rate_burst), (200.0, 400.0));
        assert!(c.rate_overrides.is_empty());
        assert_eq!(c.api_keys, vec![("alice".to_string(), "k1".to_string()), ("key1".to_string(), "k2".to_string())]);
    }

    #[test]
    fn flags_override_env_and_validate() {
        let c = p(&[("SHARINGAN_BLUEPRINT", "/x"), ("SHARINGAN_BUDGET_MS", "100")], &["--no-auth", "--budget-ms", "250", "--set=flop=0,iters=10"]).unwrap();
        assert_eq!((c.budget_ms, c.auth, c.sets.as_str()), (250, false, "flop=0,iters=10"));
        assert!(p(&[], &["--no-auth"]).is_err(), "blueprint required");
        assert!(p(&[("SHARINGAN_BLUEPRINT", "/x")], &[]).is_err(), "keys required");
        let c = p(&[("SHARINGAN_BLUEPRINT", "/x"), ("SHARINGAN_ARENA_AUTH_URL", "http://127.0.0.1:8080/")], &[]).unwrap();
        assert_eq!((c.arena_auth_url.as_deref(), c.public_rps, c.public_burst), (Some("http://127.0.0.1:8080"), 10.0, 20.0), "arena tokens alone are enough");
        assert!(p(&[("SHARINGAN_BLUEPRINT", "/x")], &["--no-auth", "--bogus"]).is_err());
        assert!(p(&[("SHARINGAN_BLUEPRINT", "/x")], &["--no-auth", "--budget-ms", "abc"]).is_err());
    }

    #[test]
    fn per_key_rate_overrides() {
        let c = p(&[("SHARINGAN_BLUEPRINT", "/x"), ("SHARINGAN_API_KEYS", "alice=k1"), ("SHARINGAN_RATE_LIMITS", "alice=500:1000, bob=50")], &[]).unwrap();
        assert_eq!(c.rate_overrides.get("alice"), Some(&(500.0, 1000.0)));
        assert_eq!(c.rate_overrides.get("bob"), Some(&(50.0, 100.0)));
        assert!(p(&[("SHARINGAN_BLUEPRINT", "/x"), ("SHARINGAN_API_KEYS", "a=k"), ("SHARINGAN_RATE_LIMITS", "alice")], &[]).is_err());
    }
}
