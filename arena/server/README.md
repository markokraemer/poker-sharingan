# sharingan-arena: the table server

REST + WebSocket server implementing [docs/TABLE_API.md](../../docs/TABLE_API.md). One crate, binary `sharingan-arena`.
Rules come from `engine/nlhe`, house bots and AIVAT from `arena/bench`. Storage behind the `Store` trait (`src/store.rs`): SQLite by default,
Postgres (`src/pgstore.rs`, migrations in `migrations/`) with `ARENA_DATABASE_URL`.

## Run

```bash
cargo run --release -p sharingan-arena -- --port 8080
```

The first start logs a generated admin token (set `ARENA_ADMIN_TOKEN` to pin it). Flags: `--port`, `--bind HOST:PORT`, `--db PATH`.

| env | default | meaning |
|---|---|---|
| `ARENA_BIND` | `127.0.0.1:8080` | listen address |
| `ARENA_DB` | `arena.db` | SQLite file (`:memory:` ok) |
| `ARENA_ADMIN_TOKEN` | random, logged | admin bearer token |
| `ARENA_BLUEPRINT` | unset | compact blueprint `.plcp`; enables house bot `house:sharingan-1` |
| `ARENA_BUCKETS` / `ARENA_SIZING` | `coarse` / `medium` | abstraction of that blueprint, e.g. `tables:data/buckets-200` |
| `ARENA_OPEN_REGISTRATION` | `true` | anyone may register a bot |
| `ARENA_ENABLE_WEBHOOKS` | `false` | allow bots with a `webhook_url` (the server then makes outbound requests) |
| `ARENA_HAND_DELAY_MS` | `300` | pause between hands on free tables |
| `ARENA_RECONNECT_GRACE_MS` | `2000` | how long a turn waits for a seat whose socket is gone |
| `ARENA_LEADERBOARD_MIN_HANDS` | `1000` | leaderboard threshold |
| `ARENA_SAVE_FREE_HANDS` | `true` | store histories of non-match tables |
| `ARENA_DATABASE_URL` | unset | `postgres://...` selects Postgres (TLS via rustls unless `sslmode=disable`); `ARENA_DB` is then ignored |
| `ARENA_DATABASE_SCHEMA` / `ARENA_DB_POOL` | unset / `8` | Postgres schema for all tables; connections |
| `ARENA_RUNNER_TOKEN` | unset | bearer token of the trusted eval runner: may start/attest hosted (verified) benchmark runs (TABLE_API.md 12b item 16) |
| `ARENA_RATE_LIMIT_RPS` / `ARENA_RATE_LIMIT_BURST` | `50` / `100` | per-token (anonymous: per-IP) request bucket; `0` = off. `ARENA_TRUST_PROXY=true` keys anonymous requests by `X-Forwarded-For` |
| `ARENA_MAX_TABLES_PER_PLAYER` | `10` | open free tables a player owns or sits at |
| `ARENA_MAX_MATCHES_PER_PLAYER` | `4` | unfinished matches/benchmark runs per player |
| `ARENA_MAX_WS_PER_PLAYER` | `4` | simultaneous `/v1/play` sockets per player |
| `ARENA_MAX_RUN_HANDS` / `ARENA_MAX_MATCH_HANDS` | `100000` / `1000000` | hands per benchmark run (players) / per match |
| `ARENA_MAX_BODY_BYTES` / `ARENA_MAX_WS_MESSAGE_BYTES` | `262144` / `65536` | request body / one client WebSocket message |
| `ARENA_SHUTDOWN_GRACE_MS` | `20000` | SIGTERM drain: how long running matches may finish before they are aborted |

Probes for a load balancer: `GET /healthz` (liveness), `GET /readyz` (200 only if the database answers and the server is not draining).
`SIGTERM`/`SIGINT` drains gracefully (see TABLE_API.md 12b item 18).

## Tests on both backends

```bash
cargo test -p sharingan-arena                                   # SQLite (in-memory)
ARENA_TEST_DATABASE_URL=postgres://user@host:5432/db cargo test -p sharingan-arena   # same suite on Postgres, one fresh schema per test server
```
Test schemas (`t_*`, `s_*`) are left behind; use a throwaway database.

## curl tour

```bash
H=http://127.0.0.1:8080; ADMIN=...   # admin token from the log

# 1. register a bot (token shown once; only its sha256 is stored)
curl -s $H/v1/bots -d '{"name":"mybot","kind":"bot"}' -H 'content-type: application/json'

# 2. a table with you and five house bots, then connect ws://127.0.0.1:8080/v1/play?token=$TOKEN
curl -s $H/v1/tables -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' -d '{"decision_time_ms":3000}'
curl -s -X POST $H/v1/tables/$T/seats/0 -H "authorization: Bearer $TOKEN" -d '{}' -H 'content-type: application/json'
curl -s -X POST $H/v1/tables/$T/fill -H "authorization: Bearer $TOKEN" -d '{"bot":"house:tight_aggressive"}' -H 'content-type: application/json'

# 3. a duplicate match against the ladder (random, calling_station, 2x tight_aggressive, sharingan-1)
curl -s $H/v1/matches -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"participants":["'$BOT_ID'"],"field":"ladder","hands_planned":600,"duplicate":true,"seed":"my-seed"}'
curl -s -X POST $H/v1/matches/$M/start -H "authorization: Bearer $TOKEN"
curl -s $H/v1/matches/$M                       # status + standings (mbb_per_game, ci95_mbb, ...)
curl -s $H/v1/matches/$M/hands.jsonl -H "authorization: Bearer $TOKEN"
curl -s "$H/v1/leaderboard?min_hands=1000"
```

## Benchmark tracks and throughput

```bash
curl -s $H/v1/benchmarks                                    # ladder-v1, sharingan-1 (need ARENA_BLUEPRINT)
curl -s -X POST $H/v1/benchmarks/ladder-v1/runs -H "authorization: Bearer $TOKEN" \
     -H 'content-type: application/json' -d '{"lanes":4}'   # your bot, 20,000 hands by default (connect to /v1/play first)
curl -s $H/v1/benchmarks/ladder-v1/runs/$RUN                # progress + running estimate; also a WS benchmark_progress event
curl -s "$H/v1/leaderboard?track=ladder-v1"                 # vs_ladder_v1, vs_sharingan_1, verified per bot
python3 arena/server/scripts/throughput.py --mode benchmark --lanes 4 --hands 6000   # hands/s with the python SDK
```

Fast path: `"mode":"benchmark"` (tables and matches), `"lanes":N` for parallel blocks of a duplicate match, hand histories written in
batches by a background thread, TCP_NODELAY on all sockets. See TABLE_API.md 12b items 11-15.

## What is where

- `src/hand.rs`: one hand on top of `nlhe::State` (seat mapping, payloads, side-pot results, history). No I/O.
- `src/table.rs`: per-table task, decision loop, timeouts, reconnect, house bots, webhook seats.
- `src/matches.rs`: duplicate schedule, AIVAT replay (bench `Tracker` over stored hands), finalize.
- `src/benchmarks.rs`: tracks, runs, verified flag. `src/writer.rs`: batched hand writer.
- `src/stats.rs`: standings (deal as the sample) and leaderboard (inverse-variance combine).
- `src/api.rs`, `src/ws.rs`: REST and WebSocket. `src/deck.rs`: HMAC-SHA256 + ChaCha20 Fisher-Yates dealing.

## Behaviour worth knowing

- Fold when check is free becomes a check. Illegal actions get an `error` and the same `your_turn` (same `decision_id`) again until the deadline.
- `deadline_ms` includes the player's remaining time bank. A seat with no live socket at the start of its turn gets only the reconnect grace.
- Free tables deal when `min_players_to_deal` seats are filled and connected (or house) and at least one is not a house bot.
- Matches: `stack_mode` is forced to `reset`, duplicate needs `hands_planned % 6 == 0`. Deals are played in blocks of `block_deals` (12) rotation-major so a deal's rotations are far apart. Hand histories of a running match are readable only by its owner/admin. `"reveal":"match_end"` also withholds other players' cards in `hand_end` until the match ends (default follows the spec and reveals).
- Scores: AIVAT (bench `EquityEstimator` baseline; house bots that expose probabilities also get decision corrections) when `aivat` is `full`/`cards`, else plain duplicate. There is no learned blueprint value function yet. Leaderboard combines matches by inverse variance, so a score depends on the field faced.
- Sandbox tables (`"mode":"sandbox"`) take `sandbox.decks` (card prefixes per hand), `button_seat`, `stacks`.

## Tests

`cargo test -p sharingan-arena` runs the section 12 conformance vectors and the end-to-end paths in `tests/conformance.rs` against an in-process server.

## Known gaps

No webhook test and no `/events` callbacks to webhook bots; `hello{resume}` always sends a fresh snapshot (no replay); tables are in memory (matches and hands persist, running matches are aborted on restart); no rate limiting; no spectator stream; in-memory per-hand records for running matches.
