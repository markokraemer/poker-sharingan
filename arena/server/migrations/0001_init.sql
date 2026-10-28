-- Arena storage, Postgres. Values that are not queried are stored as JSON text (same layout as the SQLite store).
CREATE TABLE players(
  player_id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL, owner TEXT,
  house BOOLEAN NOT NULL DEFAULT FALSE, token_hash TEXT NOT NULL DEFAULT '',
  webhook_url TEXT, webhook_secret TEXT, created_at TEXT NOT NULL, created_seq BIGSERIAL
);
CREATE INDEX players_token ON players(token_hash);
CREATE TABLE matches(
  match_id TEXT PRIMARY KEY, status TEXT NOT NULL, body TEXT NOT NULL, created_seq BIGSERIAL
);
CREATE INDEX matches_status ON matches(status);
CREATE TABLE hands(
  hand_id TEXT PRIMARY KEY, match_id TEXT, hand_no BIGINT NOT NULL, body TEXT NOT NULL, events TEXT NOT NULL
);
CREATE INDEX hands_match ON hands(match_id, hand_no);
CREATE TABLE match_results(
  match_id TEXT NOT NULL, player_id TEXT NOT NULL, hands BIGINT NOT NULL, deals BIGINT NOT NULL,
  net_chips BIGINT NOT NULL, score_mbb DOUBLE PRECISION NOT NULL, se_mbb DOUBLE PRECISION NOT NULL, method TEXT NOT NULL,
  PRIMARY KEY(match_id, player_id)
);
CREATE INDEX results_player ON match_results(player_id);
CREATE TABLE benchmark_runs(
  run_id TEXT PRIMARY KEY, track TEXT NOT NULL, player_id TEXT NOT NULL, body TEXT NOT NULL, created_seq BIGSERIAL
);
CREATE INDEX runs_track ON benchmark_runs(track, player_id);
