//! Storage behind a small trait: SQLite here (the default; one connection behind a mutex, WAL mode)
//! and Postgres in `pgstore.rs` (`ARENA_DATABASE_URL`). Values that are not queried are stored as JSON.

use crate::error::{ApiError, Res};
use crate::model::*;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::sync::Mutex;

pub trait Store: Send + Sync {
    /// Cheap liveness check of the backing database (readiness probe).
    fn ping(&self) -> Res<()>;

    fn put_player(&self, p: &PlayerRecord) -> Res<()>;
    fn get_player(&self, id: &str) -> Res<Option<PlayerRecord>>;
    fn player_by_token_hash(&self, hash: &str) -> Res<Option<PlayerRecord>>;
    fn list_players(&self, limit: u32) -> Res<Vec<PlayerRecord>>;

    fn put_match(&self, m: &MatchRecord) -> Res<()>;
    fn get_match(&self, id: &str) -> Res<Option<MatchRecord>>;
    fn list_matches(&self, limit: u32) -> Res<Vec<MatchRecord>>;
    /// Mark matches left `running`/`paused` by a previous process as `aborted`.
    fn abort_stale_matches(&self) -> Res<u64>;

    fn append_hand(&self, h: &HandHistory, events: &[Value]) -> Res<()>;
    /// Insert many hands in one transaction.
    fn append_hands(&self, items: &[(HandHistory, Vec<Value>)]) -> Res<()> {
        for (h, e) in items {
            self.append_hand(h, e)?;
        }
        Ok(())
    }
    fn get_hand(&self, id: &str) -> Res<Option<HandHistory>>;
    fn get_hand_events(&self, id: &str) -> Res<Option<Vec<Value>>>;
    fn list_hands(&self, match_id: &str, after_hand_no: u32, limit: u32) -> Res<Vec<HandHistory>>;
    fn count_hands(&self, match_id: &str) -> Res<u64>;

    fn put_run(&self, r: &RunRecord) -> Res<()>;
    fn get_run(&self, id: &str) -> Res<Option<RunRecord>>;
    /// Newest first.
    fn list_runs(&self, track: Option<&str>, player_id: Option<&str>, limit: u32) -> Res<Vec<RunRecord>>;

    fn put_results(&self, match_id: &str, rows: &[StoredResult]) -> Res<()>;
    fn all_results(&self) -> Res<Vec<StoredResult>>;
    fn results_of_player(&self, player_id: &str) -> Res<Vec<StoredResult>>;

    /// Admin cleanup: remove a match with its hands and results.
    fn delete_match_data(&self, match_id: &str) -> Res<()>;
    fn delete_run(&self, run_id: &str) -> Res<()>;
    fn delete_player(&self, player_id: &str) -> Res<()>;
}

pub struct SqliteStore {
    conn: Mutex<Connection>,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS players(
  player_id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL, owner TEXT,
  house INTEGER NOT NULL DEFAULT 0, token_hash TEXT NOT NULL DEFAULT '',
  webhook_url TEXT, webhook_secret TEXT, created_at TEXT NOT NULL, created_seq INTEGER
);
CREATE INDEX IF NOT EXISTS players_token ON players(token_hash);
CREATE TABLE IF NOT EXISTS matches(
  match_id TEXT PRIMARY KEY, status TEXT NOT NULL, body TEXT NOT NULL, created_seq INTEGER
);
CREATE TABLE IF NOT EXISTS hands(
  hand_id TEXT PRIMARY KEY, match_id TEXT, hand_no INTEGER NOT NULL, body TEXT NOT NULL, events TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS hands_match ON hands(match_id, hand_no);
CREATE TABLE IF NOT EXISTS match_results(
  match_id TEXT NOT NULL, player_id TEXT NOT NULL, hands INTEGER NOT NULL, deals INTEGER NOT NULL,
  net_chips INTEGER NOT NULL, score_mbb REAL NOT NULL, se_mbb REAL NOT NULL, method TEXT NOT NULL,
  PRIMARY KEY(match_id, player_id)
);
CREATE INDEX IF NOT EXISTS results_player ON match_results(player_id);
CREATE TABLE IF NOT EXISTS benchmark_runs(
  run_id TEXT PRIMARY KEY, track TEXT NOT NULL, player_id TEXT NOT NULL, body TEXT NOT NULL, created_seq INTEGER
);
CREATE INDEX IF NOT EXISTS runs_track ON benchmark_runs(track, player_id);
"#;

impl SqliteStore {
    /// `path` may be a file or `:memory:`.
    pub fn open(path: &str) -> Res<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(SqliteStore { conn: Mutex::new(conn) })
    }
    fn c(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn player_from_row(r: &rusqlite::Row) -> rusqlite::Result<PlayerRecord> {
    Ok(PlayerRecord {
        player_id: r.get(0)?,
        name: r.get(1)?,
        kind: r.get(2)?,
        owner: r.get(3)?,
        house: r.get::<_, i64>(4)? != 0,
        token_hash: r.get(5)?,
        webhook_url: r.get(6)?,
        webhook_secret: r.get(7)?,
        created_at: r.get(8)?,
    })
}

const PCOLS: &str = "player_id,name,kind,owner,house,token_hash,webhook_url,webhook_secret,created_at";

pub(crate) fn parse<T: serde::de::DeserializeOwned>(s: String) -> Res<T> {
    serde_json::from_str(&s).map_err(|e| ApiError::internal(format!("corrupt row: {e}")))
}

impl Store for SqliteStore {
    fn ping(&self) -> Res<()> {
        self.c().query_row("SELECT 1", [], |r| r.get::<_, i64>(0))?;
        Ok(())
    }
    fn put_player(&self, p: &PlayerRecord) -> Res<()> {
        self.c().execute(
            "INSERT INTO players(player_id,name,kind,owner,house,token_hash,webhook_url,webhook_secret,created_at,created_seq)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,(SELECT COALESCE(MAX(created_seq),0)+1 FROM players))
             ON CONFLICT(player_id) DO UPDATE SET name=?2,kind=?3,owner=?4,house=?5,token_hash=?6,webhook_url=?7,webhook_secret=?8",
            params![p.player_id, p.name, p.kind, p.owner, p.house as i64, p.token_hash, p.webhook_url, p.webhook_secret, p.created_at],
        )?;
        Ok(())
    }
    fn get_player(&self, id: &str) -> Res<Option<PlayerRecord>> {
        Ok(self
            .c()
            .query_row(&format!("SELECT {PCOLS} FROM players WHERE player_id=?1"), [id], player_from_row)
            .optional()?)
    }
    fn player_by_token_hash(&self, hash: &str) -> Res<Option<PlayerRecord>> {
        if hash.is_empty() {
            return Ok(None);
        }
        Ok(self
            .c()
            .query_row(&format!("SELECT {PCOLS} FROM players WHERE token_hash=?1"), [hash], player_from_row)
            .optional()?)
    }
    fn list_players(&self, limit: u32) -> Res<Vec<PlayerRecord>> {
        let c = self.c();
        let mut st = c.prepare(&format!("SELECT {PCOLS} FROM players ORDER BY created_seq LIMIT ?1"))?;
        let rows = st.query_map([limit], player_from_row)?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn put_match(&self, m: &MatchRecord) -> Res<()> {
        let body = serde_json::to_string(m)?;
        self.c().execute(
            "INSERT INTO matches(match_id,status,body,created_seq) VALUES(?1,?2,?3,(SELECT COALESCE(MAX(created_seq),0)+1 FROM matches))
             ON CONFLICT(match_id) DO UPDATE SET status=?2, body=?3",
            params![m.match_id, m.status, body],
        )?;
        Ok(())
    }
    fn get_match(&self, id: &str) -> Res<Option<MatchRecord>> {
        let s: Option<String> =
            self.c().query_row("SELECT body FROM matches WHERE match_id=?1", [id], |r| r.get(0)).optional()?;
        s.map(parse).transpose()
    }
    fn list_matches(&self, limit: u32) -> Res<Vec<MatchRecord>> {
        let c = self.c();
        let mut st = c.prepare("SELECT body FROM matches ORDER BY created_seq DESC LIMIT ?1")?;
        let rows = st.query_map([limit], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(parse).collect()
    }
    fn abort_stale_matches(&self) -> Res<u64> {
        let ids: Vec<String> = {
            let c = self.c();
            let mut st = c.prepare("SELECT match_id FROM matches WHERE status IN ('running','paused')")?;
            let v = st.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
            v
        };
        for id in &ids {
            if let Some(mut m) = self.get_match(id)? {
                m.status = "aborted".into();
                self.put_match(&m)?;
            }
        }
        Ok(ids.len() as u64)
    }

    fn append_hand(&self, h: &HandHistory, events: &[Value]) -> Res<()> {
        let body = serde_json::to_string(h)?;
        let ev = serde_json::to_string(events)?;
        self.c().execute(
            "INSERT OR REPLACE INTO hands(hand_id,match_id,hand_no,body,events) VALUES(?1,?2,?3,?4,?5)",
            params![h.hand_id, h.match_id, h.hand_no, body, ev],
        )?;
        Ok(())
    }
    fn append_hands(&self, items: &[(HandHistory, Vec<Value>)]) -> Res<()> {
        let mut c = self.c();
        let tx = c.transaction()?;
        {
            let mut st = tx.prepare_cached("INSERT OR REPLACE INTO hands(hand_id,match_id,hand_no,body,events) VALUES(?1,?2,?3,?4,?5)")?;
            for (h, e) in items {
                let body = serde_json::to_string(h)?;
                let ev = if e.is_empty() { "[]".to_string() } else { serde_json::to_string(e)? };
                st.execute(params![h.hand_id, h.match_id, h.hand_no, body, ev])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    fn put_run(&self, r: &RunRecord) -> Res<()> {
        let body = serde_json::to_string(r)?;
        self.c().execute(
            "INSERT INTO benchmark_runs(run_id,track,player_id,body,created_seq) VALUES(?1,?2,?3,?4,(SELECT COALESCE(MAX(created_seq),0)+1 FROM benchmark_runs))
             ON CONFLICT(run_id) DO UPDATE SET body=?4",
            params![r.run_id, r.track, r.player_id, body],
        )?;
        Ok(())
    }
    fn get_run(&self, id: &str) -> Res<Option<RunRecord>> {
        let s: Option<String> = self.c().query_row("SELECT body FROM benchmark_runs WHERE run_id=?1", [id], |r| r.get(0)).optional()?;
        s.map(parse).transpose()
    }
    fn list_runs(&self, track: Option<&str>, player_id: Option<&str>, limit: u32) -> Res<Vec<RunRecord>> {
        let c = self.c();
        let mut st = c.prepare("SELECT body FROM benchmark_runs WHERE (?1 IS NULL OR track=?1) AND (?2 IS NULL OR player_id=?2) ORDER BY created_seq DESC LIMIT ?3")?;
        let rows = st.query_map(params![track, player_id, limit], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(parse).collect()
    }
    fn get_hand(&self, id: &str) -> Res<Option<HandHistory>> {
        let s: Option<String> =
            self.c().query_row("SELECT body FROM hands WHERE hand_id=?1", [id], |r| r.get(0)).optional()?;
        s.map(parse).transpose()
    }
    fn get_hand_events(&self, id: &str) -> Res<Option<Vec<Value>>> {
        let s: Option<String> =
            self.c().query_row("SELECT events FROM hands WHERE hand_id=?1", [id], |r| r.get(0)).optional()?;
        s.map(parse).transpose()
    }
    fn list_hands(&self, match_id: &str, after: u32, limit: u32) -> Res<Vec<HandHistory>> {
        let c = self.c();
        let mut st = c.prepare("SELECT body FROM hands WHERE match_id=?1 AND hand_no>?2 ORDER BY hand_no LIMIT ?3")?;
        let rows = st.query_map(params![match_id, after, limit], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(parse).collect()
    }
    fn count_hands(&self, match_id: &str) -> Res<u64> {
        Ok(self.c().query_row("SELECT COUNT(*) FROM hands WHERE match_id=?1", [match_id], |r| r.get::<_, i64>(0))? as u64)
    }

    fn put_results(&self, match_id: &str, rows: &[StoredResult]) -> Res<()> {
        let mut c = self.c();
        let tx = c.transaction()?;
        tx.execute("DELETE FROM match_results WHERE match_id=?1", [match_id])?;
        for r in rows {
            tx.execute(
                "INSERT INTO match_results(match_id,player_id,hands,deals,net_chips,score_mbb,se_mbb,method) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![r.match_id, r.player_id, r.hands as i64, r.deals as i64, r.net_chips, r.score_mbb, r.se_mbb, r.method],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    fn all_results(&self) -> Res<Vec<StoredResult>> {
        self.results_where("")
    }
    fn results_of_player(&self, player_id: &str) -> Res<Vec<StoredResult>> {
        let c = self.c();
        let mut st = c.prepare("SELECT match_id,player_id,hands,deals,net_chips,score_mbb,se_mbb,method FROM match_results WHERE player_id=?1")?;
        let rows = st.query_map([player_id], result_from_row)?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn delete_match_data(&self, match_id: &str) -> Res<()> {
        let mut c = self.c();
        let tx = c.transaction()?;
        tx.execute("DELETE FROM hands WHERE match_id=?1", [match_id])?;
        tx.execute("DELETE FROM match_results WHERE match_id=?1", [match_id])?;
        tx.execute("DELETE FROM matches WHERE match_id=?1", [match_id])?;
        tx.commit()?;
        Ok(())
    }
    fn delete_run(&self, run_id: &str) -> Res<()> {
        self.c().execute("DELETE FROM benchmark_runs WHERE run_id=?1", [run_id])?;
        Ok(())
    }
    fn delete_player(&self, player_id: &str) -> Res<()> {
        let mut c = self.c();
        let tx = c.transaction()?;
        tx.execute("DELETE FROM match_results WHERE player_id=?1", [player_id])?;
        tx.execute("DELETE FROM benchmark_runs WHERE player_id=?1", [player_id])?;
        tx.execute("DELETE FROM players WHERE player_id=?1", [player_id])?;
        tx.commit()?;
        Ok(())
    }
}

fn result_from_row(r: &rusqlite::Row) -> rusqlite::Result<StoredResult> {
    Ok(StoredResult {
        match_id: r.get(0)?,
        player_id: r.get(1)?,
        hands: r.get::<_, i64>(2)? as u64,
        deals: r.get::<_, i64>(3)? as u64,
        net_chips: r.get(4)?,
        score_mbb: r.get(5)?,
        se_mbb: r.get(6)?,
        method: r.get(7)?,
    })
}

impl SqliteStore {
    fn results_where(&self, _w: &str) -> Res<Vec<StoredResult>> {
        let c = self.c();
        let mut st = c.prepare("SELECT match_id,player_id,hands,deals,net_chips,score_mbb,se_mbb,method FROM match_results")?;
        let rows = st.query_map([], result_from_row)?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
