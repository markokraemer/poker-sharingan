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

`{botName, language, entry, artifactTgzBase64, requirementsFile?, track, hands?, lanes?, idempotencyKey, contact?}` -> `202 {id, status, hands, deduped}`
(`200` with `deduped: true` for a repeated key; the same key with a different track, size, name or entry is `409 idempotency_conflict`).
snake_case field names (`bot_name`, `artifact_tgz_base64`, `idempotency_key`, ...) are accepted as well. Every error is
`{error: {code, message, details}}`. Aborting a job that is already over is `409 job_not_abortable`. Free mode also caps running jobs
per client address across all its bots (`FREE_MAX_CONCURRENT_JOBS_PER_IP`, `429 quota_concurrent_ip`).

Checks run cheapest first and **before** the quota, so a malformed request is always a `400` and never a `429`/`402`:
JSON body, field types, `hands` range, `contact` (optional email or X handle, max 200 characters), valid base64, the upload is a gzip tarball, `entry` (and `requirementsFile`) exist in it,
and only then the per-player quota (`429`) or credit reservation (`402`).

**Hands**: the request must be in `MIN_HANDS`..`MAX_HANDS` (default 600..200,000); the number actually played is **rounded up to a multiple
of 6** (whole deals), and that rounded value is what every response echoes (`hands: 601` is answered as `hands: 606`, matching `hands_total`;
quotas and prices use it too). So: rounded up to a multiple of 6, min 600, max 200,000 (a request of 200,000 plays 200,004).

### Job view

A public player (arena bot token) sees only: `id, status, bot_name, track, hands, hands_total, hands_done, mbb_per_game, ci95_mbb, verified,
forfeit_hands, error, bot_log_tail, run_id, arena_player_id, deals_seed, created_at, finished_at`. Failures on our side show a generic
`error`. The service token (`RUNNER_AUTH_TOKEN`) still gets the full view (sandbox, cost and ledger fields, and `contact`). `contact` is never in the public view.

`error` for a failing bot is one clear first line (for example `the bot never answered a decision (600/600 hands forfeited); nothing was
submitted.`) followed by one de-duplicated traceback / log excerpt. Progress lines are removed, and a forfeited or rejected run never
shows a score line. `bot_log_tail` is de-duplicated the same way.

## Kept bots and the competition final run

Every accepted job keeps its upload in Postgres (`job_artifacts`). `GET /v1/jobs/:id/artifact` (service token only, `403` for players)
returns the exact `.tar.gz` that was submitted. This is what makes the [competition](../../../COMPETITION.md) final run possible:

1. Rank: at the deadline, save the non-house rows of `GET https://arena.sharingan.poker/v1/leaderboard`, in its default order (score vs SHARINGAN-1). Take about the top 10.
2. For each `player_id`: `GET /v1/jobs?userId=<player_id>&limit=50` with the service token. Pick the latest job created before the deadline that is
   `finished`, `verified` and on `track: sharingan-1`. Its `contact` (or an earlier job's) is how to reach the owner.
3. `GET /v1/jobs/<job id>/artifact > bot.tgz`.
4. Re-run with the service token: `POST /v1/jobs` with `userId: "final-<player_id>"` (one quota per bot), the original `botName`
   prefixed `final-`, `language`, `entry`, `requirementsFile`, `track: "sharingan-1"`, `hands: 200000`,
   `idempotencyKey: "final-<player_id>"` and `artifactTgzBase64` of `bot.tgz`. Every job gets new secret deals.
5. Rank the final jobs by `mbb_per_game`. They show on the leaderboard as `final-<name>` rows; remove them afterwards with the arena's
   admin `DELETE /v1/bots/{id}` if wanted.

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
