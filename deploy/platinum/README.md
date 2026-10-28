# Hosted Poker Sharingan: Platinum + Stripe

The hosted product: players buy credits (Stripe), submit a bot, the runner plays it against the house bots inside an isolated
sandbox, and the arena stores a **verified** score. Everything below runs end to end on one machine without Platinum or Stripe
credentials (`deploy/local/e2e.sh`); going live is configuration plus the Platinum gaps listed at the end.
The hosting design and the full Platinum gap analysis are in [DESIGN.md](DESIGN.md).

| Path | What |
|---|---|
| `docker/Dockerfile` | `arena`, `serve`, `core` (arena + serve in one VM) targets. Built and run: yes. Non-root, tini, `ROLE` entrypoint |
| `docker/Dockerfile.bot` | the **match-worker sandbox** image: `sharingan-arena` binary + python/node SDKs. Takes the arena image as `ARENA_IMAGE` |
| `docker/Dockerfile.runner` | the runner service (Node 22, TypeScript via type stripping) |
| `runner/` | credit ledger, Stripe webhook, job API, sandbox orchestration. `npm run typecheck`, `npm test` (93 tests, 106 with Postgres) |
| `runner/src/platinum/` | `PlatinumClient` interface; `real.ts` (@platinum-dev/sdk) and `local.ts` (containers via the Docker Engine API) |
| `templates/`, `bot/`, `edge/` | Platinum template specs, in-sandbox launchers, Cloudflare Worker for `arena.sharingan.poker` |
| `../local/` | `compose.yml` (postgres + arena + runner), `e2e.sh`, the example bot |

## Run it locally

Needs podman (rootless is fine) or docker, a compose (docker-compose v2, `docker compose` or podman-compose), curl, openssl, python3.
Nothing leaves the machine: Stripe checkout is simulated and the webhook is signed with a fake secret by the script itself.

```bash
# on the dev box (podman socket + docker-compose v2 installed, see "Box setup" below)
E2E_CPUSET=0-15 deploy/local/e2e.sh                 # worker mode (default): ~2 min cold (image builds), ~25 s warm
E2E_BUILD=0 E2E_MODE=connect deploy/local/e2e.sh    # remote-bot mode (WebSocket + hosted:true run)
```

What it does and asserts (23 checks per mode, plus 6 for the broken-bot case in worker mode):

1. builds `arena`, `bot` (worker sandbox) and `runner` images, starts postgres + arena (Postgres store, blueprint mounted read-only,
   `ARENA_RUNNER_TOKEN` + `ARENA_WORKER_KEYS` set) + runner (`SANDBOX_BACKEND=local`);
2. submits with an empty balance -> **402 insufficient_credits**;
3. creates a (simulated) checkout session, sends a webhook signed with the **wrong** secret -> 400, then a correctly signed
   `checkout.session.completed` -> credited $10.00; replaying it is a no-op;
4. submits `deploy/local/bots/tag_bot.py` (the SDK's TightAggressive example) for 2,000 hands of `ladder-v1` (rounded to 2,004);
5. waits, then asserts: job `finished`, **`verified: true`** (job and the arena's own run record: `finished true true`), hands = 2,004,
   AIVAT score present, credits debited exactly `hands x price` (100,200 micro-USD at $0.05 per 1,000 hands), nothing left reserved,
   ledger = topup, reserve, settle, sandbox container deleted, idempotent resubmit does not charge again;
6. worker mode only: a bot that cannot import (`def act(:`) is **failed, not charged, not posted** to the leaderboard, its traceback is
   in `bot_log_tail`, and its sandbox is gone.

Last run on the dev box (worker mode, tiny test blueprint): 2,004 hands in about 3 s, 23 passed, 0 failed (see the report for the paste).
`E2E_KEEP=1` leaves the stack up (runner `127.0.0.1:18090`, arena `127.0.0.1:18080`). Only loopback is bound.

Local vs production, honestly: the sandbox is a container, not a microVM; "no network" is `NetworkMode: none`, and the connect-mode
egress allowlist is an `internal: true` network. The Platinum calls are covered by tests against the SDK with a mocked `fetch`, not
a live control plane.

### Box setup (what was installed on box A)

`sudo apt-get install -y --no-install-recommends podman uidmap slirp4netns fuse-overlayfs passt netavark aardvark-dns catatonit crun docker-compose-v2`
(no Docker daemon; `docker-compose-v2` is only the compose binary, linked as `/usr/local/bin/docker-compose`), then
`systemctl --user enable --now podman.socket`. Node 22 through nvm. Rootless images are tagged `localhost/sharingan-{arena,serve,core,bot,runner}:local`.

## How a job runs

```
POST /v1/jobs (userId, bot tarball, track, hands, idempotencyKey)
  1 ledger.reserve(price of the hands requested)            402 if short; idempotent on the key
  2 arena POST /v1/bots                                      a fresh player id for this job (owner = userId)
  3 sandbox create from the bot template, uploads the bot    worker mode: NO network. connect mode: the arena only
  4 optional pip install (PyPI allowed only for this step)   then egress goes back to the base list
  5 worker mode:  sharingan-arena run --bot "env -i ... setpriv --reuid=10002 ... sharingan pipe bot.py" --track T --hands N
                  --lanes L --out /work/result.json --sign-key-file <id=secret, root-only> --player-id P
                  runner reads result.json, POST /v1/results (ARENA_RUNNER_TOKEN) -> arena checks the HMAC against
                  ARENA_WORKER_KEYS, the frozen panel + seed + blueprint fingerprint -> run stored verified:true
     connect mode: sharingan connect (WebSocket) + POST /v1/benchmarks/T/runs {player_id, hosted:true} (runner token), poll
  6 usage(), delete() in a finally; COGS recorded (sandbox seconds x shape, max'ed with Platinum's own number, + per-hand share)
  7 ledger.settle(hands played x price); the rest of the hold is released. Our failure => charge 0
```

Charging rules (`runner/src/service.ts` `chargeFor`): charge = hands the arena says were played x `PRICE_USD_PER_1K_HANDS`
(min `MIN_JOB_CHARGE_USD`); a job that fails because of **us** (provisioning, arena down, rejected signature, unverified result)
is refunded in full; a job that fails because of the **bot** is charged for hands played (`FAILED_BOT_CHARGE_USD` flat fee if none);
a bot that never answered a decision (all hands forfeited) fails without posting a result. Hands forfeited after a mid-run crash
are dealt, scored and billed (the compute happened) and reported as `forfeit_hands`. COGS and margin are recorded per job
(`cogs_micro` vs `charge_micro`), never billed.

Crash safety: jobs and the ledger are in Postgres. On boot `recover()` fails jobs that were running (deletes their sandbox,
refunds), settles finished-but-unsettled jobs, releases stale reservations that have no job, and reaps orphaned sandboxes
(`metadata.sharingan_job`); the reaper also runs every 5 minutes.

## Runner API

Bearer `RUNNER_AUTH_TOKEN` on everything except `/healthz`, `/readyz` and the Stripe webhook (signature only). Money is
integer micro-USD (`*_micro`, with `*_usd` conveniences).

| | |
|---|---|
| `GET /v1/credits/:userId` | balance, reserved, price per 1k hands |
| `GET /v1/credits/:userId/entries` | append-only ledger entries |
| `POST /v1/credits/checkout` | `{userId, pack \| amountUsd, successUrl?, cancelUrl?}` -> `{id, url}` (Stripe Checkout, or simulated without `STRIPE_SECRET_KEY`) |
| `POST /v1/webhooks/stripe` | verifies `Stripe-Signature` (HMAC-SHA256 over `t.rawBody`, 300 s tolerance); `checkout.session.completed` and `.async_payment_succeeded` credit once per session id; other products and unpaid sessions are acknowledged and ignored |
| `GET /v1/quote?hands=N` | price of a run |
| `POST /v1/jobs` | `{userId, botName, language, entry, artifactTgzBase64, requirementsFile?, track, hands, lanes?, mode?, idempotencyKey}` -> 202 `{id}`; 200 `deduped:true` on a repeat key; 402 when short |
| `GET /v1/jobs/:id`, `GET /v1/jobs?userId=` | status, progress, score, `verified`, charge, `bot_log_tail`, sandbox state |
| `POST /v1/jobs/:id/abort` | |

## Configuration (runner)

All env, validated at boot (`runner/src/config.ts`). The ones you must set:

| env | |
|---|---|
| `RUNNER_AUTH_TOKEN` | bearer your web / billing backend uses |
| `RUNNER_DATABASE_URL` | Postgres for the ledger and jobs (separate database or schema from the arena's) |
| `ARENA_URL`, `ARENA_RUNNER_TOKEN` | the arena and its `ARENA_RUNNER_TOKEN` (not an admin token) |
| `WORKER_KEY_ID`, `WORKER_SIGN_KEY` | the key the worker signs with; the arena needs `ARENA_WORKER_KEYS=<id>=<secret>` |
| `WORKER_ENV_JSON` | env for the worker process, e.g. `{"ARENA_BLUEPRINT":"...","ARENA_BUCKETS":"tables:...","ARENA_SIZING":"medium"}` |
| `STRIPE_WEBHOOK_SECRET`, `STRIPE_SECRET_KEY` | webhook signing secret `whsec_...`; secret key (unset = simulated checkout, no network) |
| `PRICE_USD_PER_1K_HANDS` | **the price** (default 0.05 = $1 per 20,000 hands: a placeholder, decide it) |
| `SANDBOX_BACKEND` | `platinum` (default; needs `PT_TOKEN`, `PT_API_URL`, `BOT_TEMPLATE`, `PT_REGION`) or `local` |
| `CREDIT_PACKS` | JSON `[{id, price_usd, credit_usd, label}]` (defaults: $10, $50 -> $55, $200 -> $240) |

Also: `BOT_CPU`/`BOT_RAM_MB`/`BOT_DISK_GB`, `MAX_CONCURRENT_RUNS`, `RUN_DEADLINE_S`, `MIN_HANDS`/`MAX_HANDS`, `FAILED_BOT_CHARGE_USD`,
`USD_PER_VCPU_SECOND`/`USD_PER_GIB_SECOND`/`USD_PER_HAND_ARENA_SHARE` (COGS model), `DEFAULT_JOB_MODE`, `WORKER_CMD_TEMPLATE`.

## Tests

```bash
cd deploy/platinum/runner && npm install && npm run typecheck
npm test                                                    # 92 pass, the Postgres contract tests are skipped
TEST_DATABASE_URL=postgres://user:pw@127.0.0.1:55432/db npm test   # all 106, ledger + job store on Postgres (fresh schema per test)
```

Covers: Stripe signature scheme and event handling with a fake secret, checkout request shape with a fake fetch, ledger
(idempotency, concurrency, no overdraft, invariant) on memory and Postgres, the local backend against a fake Docker Engine on a
unix socket, the real SDK adapter through the SDK's `fetch` override, every job verdict (both modes) with fake sandbox + fake arena,
the service and HTTP layer (402, refunds, dedup, abort, concurrency cap, crash recovery).

## Going live

1. **Platinum.** Two orgs, as in DESIGN.md: `sharingan-bots` (runner key scoped `sandboxes:write` + `templates:write` for provisioning;
   set `PUT /v1/billing/limits` as a circuit breaker; auto top-up) and `sharingan-core` (the arena). Set `PT_API_URL`, `PT_TOKEN`, `PT_REGION`.
2. **Images.** Push `sharingan-{arena,serve,core,bot,runner}:0.1.0` to a registry Platinum can pull anonymously (GHCR, public; no weights inside),
   build the bot image from the pushed arena image (`--build-arg ARENA_IMAGE=ghcr.io/<org>/sharingan-arena:0.1.0`), replace `REPLACE_ORG` in `templates/*.json`,
   then `PT_TOKEN=... npm run provision -- sharingan-bot`. Commit `Cargo.lock` with the arena's new deps and add `--locked` back to `docker/Dockerfile`.
3. **Big-RAM host for SHARINGAN-1** (384 GB RAM, 2+ TB NVMe, KVM) running the `core` image (`ASSETS_S3_URI`, read-only AWS creds for the blueprint
   bucket, `ARENA_SIZING=medium`, `ARENA_ADMIN_TOKEN`, `ARENA_RUNNER_TOKEN`, `ARENA_WORKER_KEYS`, `ARENA_DATABASE_URL` for a managed Postgres,
   `SHARINGAN_API_KEYS`). Interim: a plain `docker run` on the rented box; move into a Platinum `bigmem` pool when G1/G2 land.
4. **DNS.** `arena.sharingan.poker` on the existing Cloudflare zone -> `edge/arena-proxy-worker.ts` (`UPSTREAM` = the arena's public URL), WebSockets pass through.
   Rate limiting in Cloudflare; the runner calls the arena directly (`ARENA_URL`) so job traffic does not pay for the edge.
5. **Stripe.** Account, products are created per session (no price ids needed). Set `STRIPE_SECRET_KEY` (test key first), add a webhook endpoint
   `https://<runner host>/v1/webhooks/stripe` for `checkout.session.completed` and `checkout.session.async_payment_succeeded`, put its signing secret in
   `STRIPE_WEBHOOK_SECRET`, set `PUBLIC_BASE_URL` (success/cancel return). Refund policy for infra failures is already automatic (credits).
   Stripe refunds / disputes (`charge.refunded`, `charge.dispute.created`) are acknowledged but do nothing: handle them by hand until policy is decided.
6. **Run the runner** (any small always-on service, not necessarily on Platinum) with the env above; expose only to your web / billing backend
   (it is bearer-token authenticated, not a player-facing API) and publicly only `/v1/webhooks/stripe`.
7. Secrets to mint: `ARENA_ADMIN_TOKEN`, `ARENA_RUNNER_TOKEN`, worker key (`openssl rand -hex 32`), `RUNNER_AUTH_TOKEN`, `SHARINGAN_API_KEYS`, DB passwords.
8. Decisions: price per 1,000 hands, credit packs, Platinum's real per-vCPU/GiB-second rates for the COGS model, free-credit grants (use a `topup` with a `grant:` event id).

## Remaining Platinum product gaps (exact)

From DESIGN.md (unchanged unless noted), ordered by what blocks launch:

| # | Gap | Blocks |
|---|---|---|
| G1/G2 | RAM per sandbox capped at 32 GiB, disk and volumes at 100 GB (`sandboxes.ts:473-475`, `config.ts:813-814`) | SHARINGAN-1 on Platinum itself (interim: external big-RAM box) |
| **G15 (new)** | **Worker sandboxes need the blueprint.** `ladder-v1`'s panel contains `house:sharingan-1`, so `sharingan-arena run` mmaps the blueprint in-process and `/v1/results` rejects a mismatched `blueprint_fingerprint`. A 140 GB mmap per sandbox is impossible on Platinum today (G1/G2/G6: no shared read-only volume with a shared page cache; two sandboxes never share guest page cache) | `ladder-v1` / `sharingan-1` workers in production. Options: run the worker inside the `core` VM (isolate the bot with bubblewrap/gVisor, weaker than a microVM), or have the worker's SHARINGAN-1 seat query `sharingan-serve` over HTTP (arena change), or a worker-sized blueprint |
| G3 | no custom domains | `arena.sharingan.poker` (interim: Cloudflare Worker) |
| G4 | no private sandbox-to-arena network | connect-mode latency/cost only; worker mode has no network at all |
| G5 | apps cannot be stateful (no volumes, always blue-green) | arena as an App with SQLite (moot with Postgres) |
| G9 | no hard per-sandbox lifetime or budget | runner-crash leak window (mitigated: guest `timeoutMs`, `auto_stop_minutes`, 5-minute reaper) |
| G10 | `usage()` 404s after delete, 60 s sampling, no tag grouping, billing endpoints need a full-access key | exact COGS (we meter ourselves; margin figures are estimates) |
| G12 | tier concurrency caps | burst capacity (raise org overrides; runner `MAX_CONCURRENT_RUNS`) |
| G7, G13, G14 | private images, `prepare` for cold start, DNS under deny-all | worker sandboxes need no DNS (no egress at all); only the pip window and connect mode do |
| (unverified) | `network:{allow_out:[], deny_out:["0.0.0.0/0"]}` is accepted and still leaves exec / file / process APIs working; `setpriv` and `env -i` inside the guest image; `files.write` then `chmod 0400` for the key; `process.start` `timeout_ms` kill semantics; `usage()` cost field name; `allow_out` hostname rules and DNS (G14) | first live run; every call shape is typed against SDK 0.9.0 and exercised against a mock transport only |

Arena-side items (not in `deploy/`, owned by the worker agent): commit `Cargo.lock` with the new dependencies; the worker key is a long-lived
HMAC secret that sits (root-only) in the sandbox for the run, so a per-job derived key (`HMAC(master, job_id)` verified by the arena) would shrink the blast radius of a
sandbox escape; `/v1/results` accepts all-forfeit results (the runner filters those out itself).

## What is mocked

| Real | Mocked / stubbed |
|---|---|
| podman, images, compose, Postgres (arena and runner), the arena, the signed match worker, `sharingan pipe` / `connect`, the ledger, the HTTP API, webhook signature verification | **Platinum** (the local backend replaces microVMs with containers; the real adapter is tested with a mock fetch), **Stripe** (fake signing secret, simulated checkout, checkout REST call tested with a fake fetch), **S3 asset fetch** (only the mounted-blueprint path ran), Cloudflare Worker |
