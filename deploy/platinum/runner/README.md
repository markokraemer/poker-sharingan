# Hosted-job runner

Runs uploaded Sharingan bots in disposable sandboxes and posts the signed result to the arena. HTTP skin: `src/app.ts`,
product logic: `src/service.ts`, one job: `src/job.ts`. Design and operations: `../README.md` and `../DESIGN.md`.

## Public API (browser-callable)

`/v1/jobs`, `/v1/jobs/:id`, `/v1/jobs/:id/abort` and `/v1/quota` are called straight from sharingan.poker with the player's arena bot
token (`Authorization: Bearer <token>`, free billing mode). Every response carries

```
access-control-allow-origin: *
access-control-allow-methods: GET, POST, DELETE, OPTIONS
access-control-allow-headers: authorization, content-type
access-control-max-age: 86400
```

and `OPTIONS` is answered `204` without authentication.

### `POST /v1/jobs`

`{botName, language, entry, artifactTgzBase64, requirementsFile?, track, hands?, lanes?, idempotencyKey, contact, profile}` -> `202 {id, status, hands, deduped}`
(`200` with `deduped: true` for a repeated key; the same key with a different track, size, name or entry is `409 idempotency_conflict`).
snake_case field names (`bot_name`, `artifact_tgz_base64`, `idempotency_key`, ...) are accepted as well. Every error is
`{error: {code, message, details}}`. Aborting a job that is already over is `409 job_not_abortable`. Free mode also caps running jobs
per client address across all its bots (`FREE_MAX_CONCURRENT_JOBS_PER_IP`, `429 quota_concurrent_ip`).

Checks run cheapest first and **before** the quota, so a malformed request is always a `400` and never a `429`/`402`:
JSON body, field types, `hands` range, `contact` (an email) and `profile` (an http(s) link to a public profile: X, LinkedIn, GitHub, a personal page; scheme optional, `@handle` means X; stored as https), both max 200 characters and **required on every player submit** (the service token may omit them), valid base64, the upload is a gzip tarball, `entry` (and `requirementsFile`) exist in it,
and only then the per-player quota (`429`) or credit reservation (`402`).

**Hands**: the request must be in `MIN_HANDS`..`MAX_HANDS` (default 600..200,000); the number actually played is **rounded up to a multiple
of 6** (whole deals), and that rounded value is what every response echoes (`hands: 601` is answered as `hands: 606`, matching `hands_total`;
quotas and prices use it too). So: rounded up to a multiple of 6, min 600, max 200,000 (a request of 200,000 plays 200,004). The competition ranks
only a verified run of the full 20,004 hands (`ARENA_RANKED_MIN_HANDS`); a shorter run scores but never ranks.

**Limits on the public arena (free mode)**: 1M hands per bot per rolling 24 h, where every accepted job counts its planned hands even if it is aborted or
fails (only our own infra failures are free); 2 jobs at a time per bot and 2 per client address (`FREE_MAX_CONCURRENT_JOBS`, `FREE_MAX_CONCURRENT_JOBS_PER_IP`);
10 submits a minute per address (`SUBMITS_PER_MINUTE_PER_IP`); a run must finish within 30 minutes (`RUN_DEADLINE_S=1800`, so roughly under 50 ms per decision
on average; a single decision may take up to 5 s). The sandbox has Python, no network, 4 CPU and 12 GB. A 20,004-hand job of a fast bot takes about 15 s.
From `COMPETITION_CLOSES_AT` (2026-11-16 00:00 UTC, i.e. after November 15, 23:59 UTC) the `sharingan-1` track takes no public submits (`403 competition_closed`);
`ladder-v1` stays open. The service token is not affected.

### Job view

A public player (arena bot token) sees only: `id, status, bot_name, track, hands, hands_total, hands_done, mbb_per_game, ci95_mbb, verified,
forfeit_hands, error, bot_log_tail, run_id, arena_player_id, artifact_sha256 (sha256 of the uploaded .tar.gz), deals_seed (once the job is over), created_at, finished_at`. Failures on our side show a generic
`error`. The service token (`RUNNER_AUTH_TOKEN`) still gets the full view (sandbox, cost and ledger fields, `contact` and `profile`). `contact` and `profile` are never in the public view.

`error` for a failing bot is one clear first line (for example `the bot never answered a decision (600/600 hands forfeited); nothing was
submitted.`) followed by one de-duplicated traceback / log excerpt. Progress lines are removed, and a forfeited or rejected run never
shows a score line. `bot_log_tail` is de-duplicated the same way.

## Kept bots and the competition final table

Every accepted job keeps its upload in Postgres (`job_artifacts`). `GET /v1/jobs/:id/artifact` (service token only, `403` for players)
returns the exact `.tar.gz` that was submitted. This is what the [competition](../../../COMPETITION.md) final table plays:

1. At the deadline (the `sharingan-1` track takes no submits from 2026-11-16 00:00 UTC), save the non-house rows of
   `GET https://arena.sharingan.poker/v1/leaderboard?track=sharingan-1` in their order (verified rows first, then score vs SHARINGAN-1). The leaders are the
   top 12 bots, one bot per person (by `profile`).
2. For each `player_id`: `GET /v1/jobs?userId=<player_id>&limit=50` with the service token and pick the latest job created before the
   deadline that is `finished`, `verified`, on `track: sharingan-1` and has `hands_done` >= 20004 (the board ignores shorter runs).
   Its `contact` reaches the owner; `profile` says who they are.
3. `GET /v1/jobs/<job id>/artifact > <player_id>.tgz` (its sha256 is the job's `artifact_sha256`).
4. Re-run: play every leader's artifact again on fresh deals, 200,004 hands each (a service-token job on `sharingan-1` with `hands: 200000`), so each score comes
   from the exact upload behind the leaderboard score. A bot qualifies when its re-run score is above 0 (it beat SHARINGAN-1). The top 6 qualifiers are the
   finalists; if fewer than 6 qualify, copies of SHARINGAN-1 take the empty seats.
5. Final table: play the six bots against each other at one table, at least 1,000,000 hands, every deal from every seat, each bot in its own sandbox
   without network, with anonymized names. The highest win rate wins.

## Fresh secret deals

Every worker-mode job gets its own seed `run:<64 lowercase hex>` (`crypto.randomBytes(32)`), written into the sandbox at
`/opt/sharingan/run.seed` with mode 0400 (root only, like `/opt/sharingan/worker.key`; the bot runs as uid 10002) and handed to the worker
as `--seed-file /opt/sharingan/run.seed`. It is never in the bot's environment or arguments. The seed is recorded on the job as
`deals_seed` only once the job is over (`finished`, `failed` or `aborted`); before that it is `null` in every view.

`WORKER_CMD_TEMPLATE` accepts `{seedfile}`; a custom template without it still gets `--seed-file` appended. The arena side (the worker's
`--seed-file` flag and acceptance of results carrying a seed) must be deployed before this runner.

## Tests

```
npm ci && npm run typecheck && npm test
```
