//! Postgres implementation of [`Store`], selected by `ARENA_DATABASE_URL=postgres://...`.
//!
//! The `Store` trait is synchronous (like the SQLite one). The blocking `postgres` client owns an
//! internal tokio runtime and must not be called from a tokio worker thread, so every connection
//! lives on its own plain OS thread and callers hand it a closure over a channel (a small fixed pool,
//! `ARENA_DB_POOL`, default 8). Callers block until the reply arrives, exactly as they do on SQLite's mutex.
//!
//! Migrations are embedded (`migrations/*.sql`), applied in order under an advisory lock and
//! recorded in `arena_migrations`, so several server replicas can start against one database.
//! `ARENA_DATABASE_SCHEMA` puts all tables in a named schema (used by the tests for isolation).

use crate::error::{ApiError, Res};
use crate::model::*;
use crate::store::{parse, Store};
use postgres::types::ToSql;
use postgres::{Client, Config, NoTls};
use serde_json::Value;
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

type Job = Box<dyn FnOnce(&mut Client) + Send>;

pub struct PgStore {
    tx: Sender<Job>,
}

/// Embedded migrations, in order. Never edit an applied one; append a new file.
const MIGRATIONS: &[(i32, &str, &str)] = &[(1, "init", include_str!("../migrations/0001_init.sql"))];

impl From<postgres::Error> for ApiError {
    fn from(e: postgres::Error) -> Self {
        ApiError::internal(format!("storage: {e}"))
    }
}

fn connect(cfg: &Config) -> Result<Client, String> {
    if matches!(cfg.get_ssl_mode(), postgres::config::SslMode::Disable) {
        return cfg.connect(NoTls).map_err(|e| e.to_string());
    }
    let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let tls = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    cfg.connect(tokio_postgres_rustls::MakeRustlsConnect::new(tls)).map_err(|e| e.to_string())
}

impl PgStore {
    pub fn open(url: &str, schema: Option<&str>, pool: usize) -> Result<Self, String> {
        let mut cfg: Config = url.parse().map_err(|e: postgres::Error| format!("ARENA_DATABASE_URL: {e}"))?;
        if let Some(s) = schema {
            if s.is_empty() || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err("ARENA_DATABASE_SCHEMA must match [A-Za-z0-9_]+".into());
            }
            // Create the schema over a throwaway connection, then pin every pooled connection to it.
            let url2 = url.to_string();
            let s2 = s.to_string();
            std::thread::spawn(move || -> Result<(), String> {
                let c: Config = url2.parse().map_err(|e: postgres::Error| e.to_string())?;
                let mut cl = connect(&c)?;
                cl.batch_execute(&format!("CREATE SCHEMA IF NOT EXISTS \"{s2}\"")).map_err(|e| e.to_string())
            })
            .join()
            .map_err(|_| "schema thread panicked".to_string())??;
            cfg.options(&format!("-c search_path={s}"));
        }
        let (tx, rx) = channel::<Job>();
        let rx: Arc<Mutex<Receiver<Job>>> = Arc::new(Mutex::new(rx));
        let n = pool.clamp(1, 64);
        let (ready_tx, ready_rx) = sync_channel::<Result<(), String>>(n);
        for i in 0..n {
            let (rx, cfg, ready) = (rx.clone(), cfg.clone(), ready_tx.clone());
            std::thread::Builder::new()
                .name(format!("pg-{i}"))
                .spawn(move || {
                    let mut client = match connect(&cfg) {
                        Ok(c) => {
                            let _ = ready.send(Ok(()));
                            c
                        }
                        Err(e) => {
                            let _ = ready.send(Err(e));
                            return;
                        }
                    };
                    loop {
                        let job = { rx.lock().unwrap_or_else(|e| e.into_inner()).recv() };
                        match job {
                            Ok(j) => j(&mut client),
                            Err(_) => break,
                        }
                    }
                })
                .map_err(|e| e.to_string())?;
        }
        for _ in 0..n {
            ready_rx.recv().map_err(|_| "postgres worker died".to_string())?.map_err(|e| format!("postgres connect: {e}"))?;
        }
        let st = PgStore { tx };
        st.run(migrate).map_err(|e| format!("postgres migrate: {e}"))?;
        Ok(st)
    }

    fn run<T: Send + 'static>(&self, f: impl FnOnce(&mut Client) -> Res<T> + Send + 'static) -> Res<T> {
        let (rtx, rrx) = sync_channel(1);
        self.tx
            .send(Box::new(move |c| {
                let _ = rtx.send(f(c));
            }))
            .map_err(|_| ApiError::internal("storage: pool closed"))?;
        rrx.recv().map_err(|_| ApiError::internal("storage: worker died"))?
    }
}

fn migrate(c: &mut Client) -> Res<()> {
    let mut tx = c.transaction()?;
    tx.batch_execute("SELECT pg_advisory_xact_lock(727274001)")?;
    tx.batch_execute("CREATE TABLE IF NOT EXISTS arena_migrations(version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at TIMESTAMPTZ NOT NULL DEFAULT now())")?;
    for (v, name, sql) in MIGRATIONS {
        let done = tx.query_opt("SELECT 1 FROM arena_migrations WHERE version=$1", &[v])?.is_some();
        if !done {
            tx.batch_execute(sql)?;
            tx.execute("INSERT INTO arena_migrations(version,name) VALUES($1,$2)", &[v, name])?;
            tracing::info!("postgres: applied migration {v:04} {name}");
        }
    }
    tx.commit()?;
    Ok(())
}

const PCOLS: &str = "player_id,name,kind,owner,house,token_hash,webhook_url,webhook_secret,created_at";
const RCOLS: &str = "match_id,player_id,hands,deals,net_chips,score_mbb,se_mbb,method";

fn player_from_row(r: &postgres::Row) -> PlayerRecord {
    PlayerRecord {
        player_id: r.get(0),
        name: r.get(1),
        kind: r.get(2),
        owner: r.get(3),
        house: r.get(4),
        token_hash: r.get(5),
        webhook_url: r.get(6),
        webhook_secret: r.get(7),
        created_at: r.get(8),
    }
}

fn result_from_row(r: &postgres::Row) -> StoredResult {
    StoredResult {
        match_id: r.get(0),
        player_id: r.get(1),
        hands: r.get::<_, i64>(2) as u64,
        deals: r.get::<_, i64>(3) as u64,
        net_chips: r.get(4),
        score_mbb: r.get(5),
        se_mbb: r.get(6),
        method: r.get(7),
    }
}

fn texts(rows: Vec<postgres::Row>) -> Vec<String> {
    rows.iter().map(|r| r.get::<_, String>(0)).collect()
}

impl Store for PgStore {
    fn ping(&self) -> Res<()> {
        self.run(|c| c.query_one("SELECT 1", &[]).map(|_| ()).map_err(Into::into))
    }

    fn put_player(&self, p: &PlayerRecord) -> Res<()> {
        let p = p.clone();
        self.run(move |c| {
            c.execute(
                "INSERT INTO players(player_id,name,kind,owner,house,token_hash,webhook_url,webhook_secret,created_at)
                 VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)
                 ON CONFLICT(player_id) DO UPDATE SET name=$2,kind=$3,owner=$4,house=$5,token_hash=$6,webhook_url=$7,webhook_secret=$8",
                &[&p.player_id, &p.name, &p.kind, &p.owner, &p.house, &p.token_hash, &p.webhook_url, &p.webhook_secret, &p.created_at],
            )?;
            Ok(())
        })
    }
    fn get_player(&self, id: &str) -> Res<Option<PlayerRecord>> {
        let id = id.to_string();
        self.run(move |c| Ok(c.query_opt(&format!("SELECT {PCOLS} FROM players WHERE player_id=$1"), &[&id])?.as_ref().map(player_from_row)))
    }
    fn player_by_token_hash(&self, hash: &str) -> Res<Option<PlayerRecord>> {
        if hash.is_empty() {
            return Ok(None);
        }
        let h = hash.to_string();
        self.run(move |c| Ok(c.query_opt(&format!("SELECT {PCOLS} FROM players WHERE token_hash=$1"), &[&h])?.as_ref().map(player_from_row)))
    }
    fn list_players(&self, limit: u32) -> Res<Vec<PlayerRecord>> {
        self.run(move |c| {
            let rows = c.query(&format!("SELECT {PCOLS} FROM players ORDER BY created_seq LIMIT $1"), &[&(limit as i64)])?;
            Ok(rows.iter().map(player_from_row).collect())
        })
    }

    fn put_match(&self, m: &MatchRecord) -> Res<()> {
        let body = serde_json::to_string(m)?;
        let (id, status) = (m.match_id.clone(), m.status.clone());
        self.run(move |c| {
            c.execute(
                "INSERT INTO matches(match_id,status,body) VALUES($1,$2,$3) ON CONFLICT(match_id) DO UPDATE SET status=$2, body=$3",
                &[&id, &status, &body],
            )?;
            Ok(())
        })
    }
    fn get_match(&self, id: &str) -> Res<Option<MatchRecord>> {
        let id = id.to_string();
        let s = self.run(move |c| Ok(c.query_opt("SELECT body FROM matches WHERE match_id=$1", &[&id])?.map(|r| r.get::<_, String>(0))))?;
        s.map(parse).transpose()
    }
    fn list_matches(&self, limit: u32) -> Res<Vec<MatchRecord>> {
        let rows = self.run(move |c| Ok(texts(c.query("SELECT body FROM matches ORDER BY created_seq DESC LIMIT $1", &[&(limit as i64)])?)))?;
        rows.into_iter().map(parse).collect()
    }
    fn abort_stale_matches(&self) -> Res<u64> {
        let ids = self.run(|c| Ok(texts(c.query("SELECT match_id FROM matches WHERE status IN ('running','paused')", &[])?)))?;
        for id in &ids {
            if let Some(mut m) = self.get_match(id)? {
                m.status = "aborted".into();
                self.put_match(&m)?;
            }
        }
        Ok(ids.len() as u64)
    }

    fn append_hand(&self, h: &HandHistory, events: &[Value]) -> Res<()> {
        self.append_hands(&[(h.clone(), events.to_vec())])
    }
    fn append_hands(&self, items: &[(HandHistory, Vec<Value>)]) -> Res<()> {
        let mut rows = Vec::with_capacity(items.len());
        for (h, e) in items {
            let ev = if e.is_empty() { "[]".to_string() } else { serde_json::to_string(e)? };
            rows.push((h.hand_id.clone(), h.match_id.clone(), h.hand_no as i64, serde_json::to_string(h)?, ev));
        }
        self.run(move |c| {
            let mut tx = c.transaction()?;
            let st = tx.prepare(
                "INSERT INTO hands(hand_id,match_id,hand_no,body,events) VALUES($1,$2,$3,$4,$5)
                 ON CONFLICT(hand_id) DO UPDATE SET match_id=$2,hand_no=$3,body=$4,events=$5",
            )?;
            for r in &rows {
                tx.execute(&st, &[&r.0, &r.1, &r.2, &r.3, &r.4])?;
            }
            tx.commit()?;
            Ok(())
        })
    }
    fn get_hand(&self, id: &str) -> Res<Option<HandHistory>> {
        let id = id.to_string();
        let s = self.run(move |c| Ok(c.query_opt("SELECT body FROM hands WHERE hand_id=$1", &[&id])?.map(|r| r.get::<_, String>(0))))?;
        s.map(parse).transpose()
    }
    fn get_hand_events(&self, id: &str) -> Res<Option<Vec<Value>>> {
        let id = id.to_string();
        let s = self.run(move |c| Ok(c.query_opt("SELECT events FROM hands WHERE hand_id=$1", &[&id])?.map(|r| r.get::<_, String>(0))))?;
        s.map(parse).transpose()
    }
    fn list_hands(&self, match_id: &str, after: u32, limit: u32) -> Res<Vec<HandHistory>> {
        let m = match_id.to_string();
        let rows = self.run(move |c| {
            Ok(texts(c.query("SELECT body FROM hands WHERE match_id=$1 AND hand_no>$2 ORDER BY hand_no LIMIT $3", &[&m, &(after as i64), &(limit as i64)])?))
        })?;
        rows.into_iter().map(parse).collect()
    }
    fn count_hands(&self, match_id: &str) -> Res<u64> {
        let m = match_id.to_string();
        self.run(move |c| Ok(c.query_one("SELECT COUNT(*) FROM hands WHERE match_id=$1", &[&m])?.get::<_, i64>(0) as u64))
    }

    fn put_run(&self, r: &RunRecord) -> Res<()> {
        let body = serde_json::to_string(r)?;
        let (id, track, pid) = (r.run_id.clone(), r.track.clone(), r.player_id.clone());
        self.run(move |c| {
            c.execute(
                "INSERT INTO benchmark_runs(run_id,track,player_id,body) VALUES($1,$2,$3,$4) ON CONFLICT(run_id) DO UPDATE SET body=$4",
                &[&id, &track, &pid, &body],
            )?;
            Ok(())
        })
    }
    fn get_run(&self, id: &str) -> Res<Option<RunRecord>> {
        let id = id.to_string();
        let s = self.run(move |c| Ok(c.query_opt("SELECT body FROM benchmark_runs WHERE run_id=$1", &[&id])?.map(|r| r.get::<_, String>(0))))?;
        s.map(parse).transpose()
    }
    fn list_runs(&self, track: Option<&str>, player_id: Option<&str>, limit: u32) -> Res<Vec<RunRecord>> {
        let (t, p) = (track.map(str::to_string), player_id.map(str::to_string));
        let rows = self.run(move |c| {
            let args: [&(dyn ToSql + Sync); 3] = [&t, &p, &(limit as i64)];
            Ok(texts(c.query(
                "SELECT body FROM benchmark_runs WHERE ($1::text IS NULL OR track=$1) AND ($2::text IS NULL OR player_id=$2) ORDER BY created_seq DESC LIMIT $3",
                &args,
            )?))
        })?;
        rows.into_iter().map(parse).collect()
    }

    fn put_results(&self, match_id: &str, rows: &[StoredResult]) -> Res<()> {
        let m = match_id.to_string();
        let rows = rows.to_vec();
        self.run(move |c| {
            let mut tx = c.transaction()?;
            tx.execute("DELETE FROM match_results WHERE match_id=$1", &[&m])?;
            for r in &rows {
                tx.execute(
                    "INSERT INTO match_results(match_id,player_id,hands,deals,net_chips,score_mbb,se_mbb,method) VALUES($1,$2,$3,$4,$5,$6,$7,$8)",
                    &[&r.match_id, &r.player_id, &(r.hands as i64), &(r.deals as i64), &r.net_chips, &r.score_mbb, &r.se_mbb, &r.method],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }
    fn all_results(&self) -> Res<Vec<StoredResult>> {
        self.run(|c| Ok(c.query(&format!("SELECT {RCOLS} FROM match_results"), &[])?.iter().map(result_from_row).collect()))
    }
    fn results_of_player(&self, player_id: &str) -> Res<Vec<StoredResult>> {
        let p = player_id.to_string();
        self.run(move |c| Ok(c.query(&format!("SELECT {RCOLS} FROM match_results WHERE player_id=$1"), &[&p])?.iter().map(result_from_row).collect()))
    }

    fn delete_match_data(&self, match_id: &str) -> Res<()> {
        let m = match_id.to_string();
        self.run(move |c| {
            let mut tx = c.transaction()?;
            tx.execute("DELETE FROM hands WHERE match_id=$1", &[&m])?;
            tx.execute("DELETE FROM match_results WHERE match_id=$1", &[&m])?;
            tx.execute("DELETE FROM matches WHERE match_id=$1", &[&m])?;
            tx.commit()?;
            Ok(())
        })
    }
    fn delete_run(&self, run_id: &str) -> Res<()> {
        let r = run_id.to_string();
        self.run(move |c| {
            c.execute("DELETE FROM benchmark_runs WHERE run_id=$1", &[&r])?;
            Ok(())
        })
    }
    fn delete_player(&self, player_id: &str) -> Res<()> {
        let p = player_id.to_string();
        self.run(move |c| {
            let mut tx = c.transaction()?;
            tx.execute("DELETE FROM match_results WHERE player_id=$1", &[&p])?;
            tx.execute("DELETE FROM benchmark_runs WHERE player_id=$1", &[&p])?;
            tx.execute("DELETE FROM players WHERE player_id=$1", &[&p])?;
            tx.commit()?;
            Ok(())
        })
    }
}
