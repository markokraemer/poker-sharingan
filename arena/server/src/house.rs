//! House bots: server-side players built on the bench Policy API.

use crate::config::ServerConfig;
use crate::error::Res;
use crate::model::PlayerRecord;
use crate::store::Store;
use crate::util::now_iso;
use bench::policy::PolicyFactory;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Default)]
pub struct HouseRegistry {
    pub bots: BTreeMap<String, Arc<dyn PolicyFactory>>,
    /// sha256 of the first 4 MiB of the blueprint file plus its size; recorded with benchmark runs.
    pub blueprint_fp: Option<String>,
}

pub fn house_id(key: &str) -> String {
    format!("house:{key}")
}

impl HouseRegistry {
    pub fn build(cfg: &ServerConfig) -> Result<Self, String> {
        let mut bots: BTreeMap<String, Arc<dyn PolicyFactory>> = BTreeMap::new();
        // (public name, bench builtin)
        for (public, n) in [
            ("random", "random"),
            ("calling_station", "station"),
            ("tight_aggressive", "tag"),
            ("nit", "nit"),
            ("lag", "lag"),
            ("maniac", "maniac"),
        ] {
            bots.insert(public.into(), bench::builtin::builtin(n).ok_or(format!("builtin {n}"))?);
        }
        for n in ["chart6", "chart6-loose", "chart6-tight"] {
            bots.insert(n.into(), Arc::new(bench::chart::Chart6::by_name(n).ok_or(format!("chart {n}"))?));
        }
        let mut blueprint_fp = None;
        if let Some(path) = &cfg.blueprint {
            blueprint_fp = Some(fingerprint(path)?);
            let b = bench::bp::bucketer_by_name(&cfg.buckets).ok_or(format!("unknown bucketer {}", cfg.buckets))?;
            let s = bench::bp::sizing_by_name(&cfg.sizing).ok_or(format!("unknown sizing {}", cfg.sizing))?;
            let p = bench::compact::CompactPolicy::open(path, b, s)?;
            bots.insert("sharingan-1".into(), Arc::new(p));
        }
        Ok(HouseRegistry { bots, blueprint_fp })
    }

    /// Resolve `house:tight_aggressive` (also bare names and the short aliases `tag`,
    /// `station`, `blueprint`) to a registry key.
    pub fn key_of(&self, s: &str) -> Option<String> {
        let k = s.strip_prefix("house:").or_else(|| s.strip_prefix("house-")).unwrap_or(s);
        let k = match k {
            "tag" => "tight_aggressive",
            "station" => "calling_station",
            "blueprint" => "sharingan-1",
            o => o,
        };
        self.bots.contains_key(k).then(|| k.to_string())
    }

    /// True if `s` names a house bot (configured or not).
    pub fn looks_like_house(s: &str) -> bool {
        s.starts_with("house:") || s.starts_with("house-")
    }

    /// Like `key_of` but with a clear error for unknown or unconfigured bots.
    pub fn resolve(&self, s: &str) -> crate::error::Res<String> {
        self.key_of(s).ok_or_else(|| {
            let k = s.trim_start_matches("house:").trim_start_matches("house-");
            if k == "sharingan-1" || k == "blueprint" {
                crate::error::ApiError::bad_request("house:sharingan-1 is not available: no blueprint configured (set ARENA_BLUEPRINT)")
            } else {
                crate::error::ApiError::bad_request(format!(
                    "unknown house bot {s}; available: {}",
                    self.bots.keys().map(|k| format!("house:{k}")).collect::<Vec<_>>().join(", ")
                ))
            }
        })
    }

    /// The `ladder` field: random, calling_station, 2x tight_aggressive, sharingan-1.
    pub fn ladder(&self) -> crate::error::Res<Vec<String>> {
        let v = ["random", "calling_station", "tight_aggressive", "tight_aggressive", "sharingan-1"];
        v.iter().map(|k| self.resolve(k).map(|k| house_id(&k))).collect()
    }

    pub fn get(&self, key: &str) -> Option<Arc<dyn PolicyFactory>> {
        self.bots.get(key).cloned()
    }

    /// Make sure every house bot exists as a (token-less) player record.
    pub fn register_players(&self, store: &dyn Store) -> Res<()> {
        for k in self.bots.keys() {
            let id = house_id(k);
            if store.get_player(&id)?.is_none() {
                store.put_player(&PlayerRecord {
                    player_id: id,
                    name: format!("house:{k}"),
                    kind: "bot".into(),
                    owner: Some("house".into()),
                    created_at: now_iso(),
                    house: true,
                    token_hash: String::new(),
                    webhook_url: None,
                    webhook_secret: None,
                })?;
            }
        }
        Ok(())
    }
}

fn fingerprint(path: &std::path::Path) -> Result<String, String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    let mut buf = vec![0u8; 4 << 20];
    let n = f.read(&mut buf).map_err(|e| e.to_string())?;
    buf.truncate(n);
    buf.extend_from_slice(&len.to_le_bytes());
    Ok(crate::util::sha256_hex(&buf)[..16].to_string())
}
