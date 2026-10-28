# Poker Sharingan on Platinum

Status: design plus untested skeletons. Nothing here was built, deployed, or run against Platinum. Repo paths marked `PT:` are in `~/Projects/kortix/platinum` (read only).

## 0. The three facts that shape everything

1. **The arena loads the blueprint itself.** `arena/server/src/house.rs:41-47` opens `ARENA_BLUEPRINT` with `CompactPolicy::open` (mmap) and serves it as `house:sharingan-1`. Both benchmark tracks (`ladder-v1`, `sharingan-1`) need it, and the 4,800 hands/s hosted path only works in-process. So the arena and the ~140 GB blueprint belong on the same machine. `sharingan-serve` (stateless `/v1/act`, with turn/river search) is a separate product; running it in the same VM lets both share one page cache (two sandboxes never share guest page cache). That is what the `core` image target is for.
2. **Platinum cannot host a 140 GB mmapped file today.** Hard caps: RAM 32 GiB per sandbox (`PT:apps/api/src/api/sandboxes.ts:473`, config clamp `PT:apps/api/src/config.ts:813`), disk 100 GB (`sandboxes.ts:475`, `templates.ts:787,876,1005,1113,1199`, `config.ts:814`), volume 100 GiB (`PT:docs/volumes.md` Limits), template size 20 GiB default (`config.ts:805`). See G1/G2.
3. **Platinum has no custom domains and no private sandbox-to-sandbox network** (`PT:apps/api/src/tlsAsk.ts`, `PT:apps/api/src/sandboxProxy.ts:105-122`, `PT:infra/03-network.sh:64-66`). `arena.sharingan.poker` needs a front door we own, and bots reach the arena over its public URL. See G3/G4.

```
 player -> sharingan.poker (Cloudflare, existing wrangler site)
        -> Stripe Checkout -> our billing service (credit ledger)
                               |  POST /jobs (credits reserved first)
                               v
                          runner service  --Platinum SDK-->  Platinum org "sharingan-bots"
                               |                               one ephemeral microVM per run
                               |                               (template sharingan-bot, egress = arena only)
                               | arena REST                         | wss://arena.sharingan.poker/v1/play
                               v                                    v
 arena.sharingan.poker --Cloudflare Worker--> Platinum org "sharingan-core"
                                              app/sandbox "core": sharingan-arena :8080 + sharingan-serve :8081
                                              bigmem pool host, 256+ GB RAM, blueprint on local disk, SQLite or Postgres
```

Two Platinum orgs on purpose: at balance <= 0 Platinum auto-stops running sandboxes after a 60 minute grace (`PT:docs/billing.md` Zero balance). Bot spend must not be able to stop the arena.

## 1. Mapping

### (a) Arena server (`ghcr.io/<org>/sharingan-arena`, `docker/Dockerfile --target arena`)

- **Shape: a Platinum App** (`/v1/apps`, `PT:apps/api/src/api/apps.ts`), source `{kind:"image"}` pinned tag, `port: 8080`, `health_path: "/healthz"` (arena route `arena/server/src/api.rs:27`). The app gives a stable URL `<slug>.apps.<domain>`, env/secret injection (`runtime_env`, secrets encrypted at rest), blue-green rollout (`PT:apps/rollout.ts` `deploy()`), rollback (`POST /v1/apps/:id/rollback`), and a GitHub push webhook if we use `{kind:"git"}` (Dockerfile in repo; `handleGitPush` in `apps.ts`, GitHub App connection `apps/github.ts`). WebSockets pass through the app/preview proxy (`sandboxProxy.ts` WS path).
- **Do not use the app path for the stateful production arena until G5 is fixed.** The rollout boots a fresh VM from the template on every deploy (`rollout.ts:128-148`, no `volumes` in `createBody`), so SQLite on the rootfs is lost on each release. Until then run the arena as a plain persistent sandbox: `sandboxes.create({template, type:"persistent", expose:[{port:8080, public:true}], env, backup_interval_min: 15})`, upgrade by snapshot + replace by hand.
- **Persistence.** `ARENA_DB=/data/arena.db`. Two honest options:
  - SQLite on the sandbox disk (persistent sandbox, `auto_stop_minutes:0`, `auto_archive_days:0`), plus a 5 minute `sqlite3 .backup` to S3. Fine for launch traffic.
  - SQLite on a Platinum volume: not recommended. A mount is a private working copy committed about every 5 s or on `fsync` (`PT:docs/volumes.md` Commit and pull); SQLite fsyncs every transaction and a second VM (rollout overlap) would fork the file.
  - **Move to Postgres** when any of: more than one arena instance, blue-green deploys needed, or hand-history volume past ~50 GB. `store.rs` already has the `Store` trait; the Postgres impl is the work (about 1-2 days). Use a managed Postgres (not on Platinum).
- **Config:** `ARENA_BIND=0.0.0.0:8080` (default is loopback, already set in the image), `ARENA_ADMIN_TOKEN` from a Platinum secret/env, `ARENA_ENABLE_WEBHOOKS=false`, `ARENA_OPEN_REGISTRATION=true` behind Cloudflare rate limits (the arena has none; README Known gaps).
- **Without blueprint** (`USE_BLUEPRINT=0`) this fits Platinum today (4 vCPU, 8 GB): `ladder-v1` and `sharingan-1` return 400, but plain duplicate matches against the other house bots work. That is Phase 1.

### (b) SHARINGAN-1 inference (needs big memory)

- Target `core` (arena + serve in one VM, entrypoint `docker/entrypoint.sh`): 32+ vCPU (search threads 16, concurrency 2, plus arena), RAM >= 256 GB (140 GB page cache plus working set), disk >= 400 GB local NVMe. The blueprint is pulled once from S3 (`ASSETS_S3_URI`, bucket `pluribus-repro-2026` or a copy) and `cat`-warmed. Image carries no weights.
- **What Platinum needs (G1, G2):** raise the RAM and disk ceilings, enrol one large host into a dedicated pool. Pools already exist and are exactly the right fence (`PT:apps/api/src/hostPools.ts`: a non-`general` pool is only used by creates that name it; `sandboxes.create({pool:"bigmem", pool_fallback:"none"})`; host joins via bootstrap token body `{"pool":"bigmem"}`, `PT:DEPLOY.md` "Dedicated host pool"). Hosts are bare metal with `/dev/kvm`; pick one with >= 384 GB RAM and 2+ TB NVMe.
- Also set `backup_interval_min: 0` (the hourly rootfs backup would tar 140 GB) and `auto_archive_days: 0` for this sandbox; the weights are re-fetchable from S3.
- **Interim (no Platinum change):** run `core` outside Platinum on one rented big-RAM Linux box with `docker run` (same image, same env), reachable at `arena.sharingan.poker` through the same Cloudflare Worker/DNS. Everything else (runner, bot sandboxes, billing) is on Platinum and only sees a URL. When G1/G2 land, move the container into a bigmem Platinum host (even the same machine: install it as a Platinum host via `PT:DEPLOY.md` host installer) with zero client changes. A weaker interim: Phase 1 arena without blueprint.

### (c) Eval runner (uploaded bots)

`runner/` (TypeScript, `@platinum-dev/sdk`; the SDK is TS/Python only, and the arena runner is a Node service). One job = one sandbox = one benchmark run:

1. Template `sharingan-bot` (`docker/Dockerfile.bot`, `templates/sharingan-bot.json`): python 3.12 + `sdk/python` + node 22 + `poker-sharingan` + two launchers. Built once by `runner/src/provision.ts` (`Template.fromImage(...).build()`, `PT:docs/templates.md`).
2. Per run: `sandboxes.create({template, type:"ephemeral", cpu:1, ram_mb:1024, network:{allowOut:[arenaHost], denyOut:[all]}, metadata:{sharingan_job}}, {waitForRunning:true})`. Cold boot about 98 ms, warm claim about 14 ms (`PT:README.md` Performance). Egress is locked to the arena host (hostname rules read TLS SNI; `PT:docs/networking.md`). The doc says the egress policy is not a hard boundary (source-IP spoofing), but the arena is public anyway and the bot only holds a throwaway token.
3. Artifact (tar.gz, <= 8 MiB; `files.write` truncates above 16 MiB, SDK README) written with `files.write`, unpacked with `sh`. Python `requirements.txt`: PyPI added to the allowlist, `pip install`, then `updateNetwork` drops it.
4. Bot started with `sbx.process.start({cmd:[setpriv ... python3 launch_bot.py], env:{ARENA_TOKEN,...}, timeoutMs})`. It connects **out** over `wss://.../v1/play`. We do not use `sharingan play` (creates its own table vs house bots) or `sharingan serve` (webhook mode: needs `ARENA_ENABLE_WEBHOOKS`, a public URL per sandbox, an extra proxy hop per decision). `bot/launch_bot.py` is a thin wrapper over `sharingan connect` (python SDK); `launch_bot.mjs` is the TypeScript equivalent.
5. Runner retries `POST /v1/benchmarks/{track}/runs` (with the bot's own token) until the arena stops answering `409 not_connected`, polls `GET .../runs/{id}` (or listens for the `benchmark_progress` WS event) until `finished`, then `usage()` and `delete()` in a `finally`.
6. **Why not fork from a snapshot.** `sandbox.fork()/clone()` cold-boots from a disk copy (about 3.5 s) and reserves the parent's full shape (`PT:docs/snapshots.md`); a template create is faster and simpler. Use snapshot+clone only for the repeat-run case where a bot has heavy installed deps: build once per bot version (`snapshot()` after `pip install`), `clone({snapshotId})` per run. Not implemented.
7. **Leak protection.** Process-level `timeoutMs` (guest-side kill), runner wall-clock deadline plus arena abort, `auto_stop_minutes:30`, and `reapOrphans()` every 5 minutes (lists running sandboxes by `metadata.sharingan_job`). G9 is the real fix.
8. **Throughput.** A trivial Python bot does about 550 hands/s with `lanes:4` (TABLE_API 12b item 12); 20,000 hands is under a minute plus decision latency. The sandbox shape is the Python bot's limit, so offer `cpu` 1/2 as a price tier.
9. **Trust.** The arena flags remote bots `verified:false` (`benchmarks.rs:159`, `verified: rec.house`). A bot run in our sandbox is not "house", so it stays unverified. Arena change (small, not done): let an admin-authenticated run request carry `hosted:true` (admin token held only by the runner) to set `verified`. Also register bots with `owner = userId` (already a field on `POST /v1/bots`, `api.rs` `RegisterReq.owner`).
10. **Cost metering.** See (d).

### (d) Credits and billing: two separate money flows

- **Players -> us (our Stripe account).** Stripe Checkout sells credit packs; webhook `checkout.session.completed` credits an append-only ledger (per user, integer micro-USD, idempotent on the Stripe event id). Before `POST /jobs` the billing service **reserves** `reserveUsd(hands)` (`runner/src/cost.ts`); after the job settles it charges `chargeUsd` and releases the rest, idempotent on `idempotencyKey`. Failed runs caused by us (provisioning, arena down) are refunded in full; a bot crash or timeout is charged for time used. Prepaid credits, no Stripe metering. The README's "buy credits, run evals" fits this.
- **Platinum -> us.** Platinum is prepaid per org (`PT:docs/billing.md`): vCPU-seconds + GiB-seconds sampled every 60 s, storage per GiB-hour, requests metered. Fund the two orgs with Platinum's own checkout/auto-top-up (`PT:apps/api/src/autoTopup.ts`) on our card, set `PUT /v1/billing/limits` (`usage_limit_usd` as circuit breaker, `usage_alert_usd` with the `org.usage_alert` webhook, `PT:docs/webhooks.md`) on the bots org.
- **Per-run cost.** `GET /v1/sandboxes/:id/usage` (SDK `sbx.usage()`, `PT:packages/sdk-ts/src/index.ts:1872`) returns uptime and cost but 404s once the sandbox is deleted (`PT:apps/api/src/api/sandboxes.ts:5640-5643`) and lags up to a 60 s cycle. So the runner reads it before delete and also meters wall-clock x shape x our rates itself, and bills `max(local, platinum)` times `MARKUP`. The fixed arena/bigmem host is recovered per hand (`USD_PER_HAND_ARENA_SHARE`), not per second. Reconcile monthly against `GET /v1/billing/usage?group=sandbox` (needs a `*` key; the runner key is scoped `sandboxes:write`, `PT:docs/api-keys.md`).

### (e) Platinum calls per step

| Step | Call | Where |
|---|---|---|
| Build templates | `Template.fromImage(img).build(client,{name,default_cpu,default_ram_mb,default_disk_gb,entrypoint})` -> `POST /v1/templates/from-image` | `PT:packages/sdk-ts/src/template.ts`, `PT:apps/api/src/api/templates.ts:866` |
| Pre-stage for fast first boot | `POST /v1/templates/:id/prepare`, `/materialize` (no typed method: `client.request`) | `PT:docs/templates.md` |
| Deploy arena as app | `client.request("POST","/v1/apps",{slug,source:{kind:"image",image},port:8080,health_path:"/healthz",runtime_env,cpu,ram_mb,region},{wait_for_state:"live"})`; redeploy `POST /v1/apps/:id/deploy`, rollback `/rollback`, logs `/logs` | `PT:apps/api/src/api/apps.ts:257,478,522,546` (no SDK wrapper; flag `flags.apps_enabled` default off, `config.ts:668`) |
| Deploy core as sandbox | `sandboxes.create({template,pool:"bigmem",pool_fallback:"none",cpu,ram_mb,disk_gb,expose:[{port:8080,public:true},{port:8081}],env,backup_interval_min:0,auto_archive_days:0,type:"persistent"},{waitForRunning:true})` | `PT:packages/sdk-ts/src/index.ts:76-170` (`SandboxSpec`) |
| Public URL | `sbx.exposedUrl(8080)` -> `https://8080-<id>.sbx.<domain>`; app: `https://<slug>.apps.<domain>` | `PT:docs/networking.md`, `apps.ts` `appUrl()` |
| Secrets/env | `env`, app `runtime_env {secret:true}`, org secrets with egress injection (not for inbound tokens) | `PT:docs/secrets.md`, `apps.ts` `encEnv` |
| Run sandbox | `sandboxes.create({template:"sharingan-bot-...", type:"ephemeral", network, metadata})` | SDK `SandboxSpec` |
| Upload bot | `sbx.files.write(path, bytes)` (<= 16 MiB), `sbx.sh(...)` | SDK `files`, `sh` |
| Run bot | `sbx.process.start({cmd,cwd,env,timeoutMs})`, `proc.logs()`, `proc.kill()` | `PT:packages/sdk-ts/src/process.ts` |
| Tighten egress | `sbx.updateNetwork({allowOut,denyOut})` | SDK index.ts:1807 |
| Meter | `sbx.usage()` | index.ts:1872 |
| Tear down | `sbx.delete()` | index.ts:1571 |
| Orphan sweep | `sandboxes.iterate({state:"running"})` + `metadata` | index.ts:1186 |
| Spend breaker | `PUT /v1/billing/limits {usage_limit_usd,usage_alert_usd}` | `PT:docs/billing.md` |
| Lifecycle events (optional) | webhooks `sandbox.state_updated`, `org.usage_alert` | `PT:docs/webhooks.md` |

## 2. Gaps in Platinum, with the minimal fix

| # | Gap (evidence) | Blocks | Minimal change |
|---|---|---|---|
| G1 | RAM per sandbox capped at 32 GiB: zod `max(32768)` at `sandboxes.ts:473` (and `:151,:6756`), config `limits.org_max_ram_mb max:32768` (`config.ts:813`), tier table `config.ts:891-900`. Reference hosts are 60 GB (`PT:README.md`). | Production blueprint (b) | Raise the zod/config max to 524288; set `limits.org_overrides[org].maxRamMb` for the core org; enrol a >= 384 GB host with `pool:"bigmem"`; check `memoryAdmission.ts` and consider disabling KSM scanning on that host (KSM over 256 GB burns CPU). |
| G2 | Disk per sandbox and template default capped at 100 GB (`sandboxes.ts:475`, `templates.ts:787,876,1005,1113,1199`, `config.ts:814`); volume max 100 GiB (`docs/volumes.md`); template size cap 20 GiB default (`config.ts:805`, per-org overridable up to 512000 MB). | Production blueprint | Raise the same maxes to >= 1024 GB behind the per-org override; weights come from S3 on first boot, so no template size change is needed. Host needs >= 1 TB NVMe free. |
| G3 | No custom domains. Only `<slug>.apps.<domain>` (`sandboxProxy.ts:105-122`) and `<port>-<id>.sbx.<domain>`; Caddy on-demand TLS is gated by `tlsAsk.ts` which says yes only to those names. | `arena.sharingan.poker` | Add `app_domains(hostname, app_id, verified_at)`, DNS TXT verification, let `lookupAppRoute` match an exact host, and let `tlsAsk` answer yes for verified hostnames. Interim: Cloudflare Worker (`edge/arena-proxy-worker.ts`). |
| G4 | No private path sandbox <-> sandbox/app: bridge-to-bridge forward is dropped (`infra/03-network.sh:64-66`); guest IPs are per-host (`10.42/16`). Bot traffic therefore hairpins through the public edge and is metered as external requests. | Latency/cost of the hot path; "internal network" requirement | Unverified whether a per-sandbox `allowOut` to a peer's `internalIp` overrides the bridge drop (test first; I believe not). Fix: org-scoped peer allow-rule (`network.allow_peers:[{sandbox, port}]`) rendered into the per-sandbox nft chain ahead of the drop, same-host only; add `colocate_with` placement so bots land on the arena's host. Cross-host needs an overlay (larger). Public path works today. |
| G5 | Apps cannot be stateful: `bootVm` passes no `volumes`, `network` or `secrets` (`rollout.ts:128-148`) and always blue-greens onto a fresh VM. | Arena as an App with SQLite | Pass `volumes` through `CreateBody` -> `createBody`; add `strategy:"recreate"` (stop old before booting new) for stateful apps. Or use Postgres and ignore this. |
| G6 | Volumes are commit/pull working copies, not a shared POSIX FS (`docs/volumes.md`). No documented mmap/lock semantics. | SQLite or weights on a volume | Document it; recommend Postgres. No Platinum change needed. |
| G7 | Template images must be pulled from a registry; I found no registry credential field on `from-image` (`templates.ts:866`) and the apps image must be a pinned tag (`apps.ts` `isPinned`). Unverified. | Private images | Publish public GHCR images (code is Apache-2.0, weights are not in the image). If private is wanted: add `registry_auth` (secret ref) to `from-image`/apps source. |
| G8 | No SDK wrapper for apps or billing (`PT:packages/sdk-parity.json`, SDK README "Not wrapped yet"); apps are flag-gated and undocumented (`config.ts:668`; no `docs/apps.md`). | Convenience only | Use `client.request`; enable via `limits.org_overrides[org].apps`. Optionally add `client.apps.*` (thin). |
| G9 | No hard per-sandbox lifetime or budget: only idle auto-stop and day-granular archive/delete (`docs/sandboxes.md` Retention). A runner crash leaves a billing sandbox. | Safe untrusted-code billing | Add `max_runtime_seconds` to `CreateBody`; reconciler deletes past `created_at + ttl` (reuse the auto-delete sweep, `reconciler.ts` near `auto_delete_days`). Optional `max_cost_usd`. Interim: guest `timeoutMs`, runner deadline, reaper. |
| G10 | Per-run cost is awkward: `/usage` 404s after delete, 60 s sampling, no tag/metadata grouping, `/v1/billing/*` needs a full-access key. | Exact metering | Keep `/v1/sandboxes/:id/usage` readable for N days after delete with a `cost_usd` field; allow `GET /v1/billing/usage?group=metadata.sharingan_job` or a read-only `billing:read` scope. Interim: own meter (`cost.ts`). |
| G11 | One prepaid balance per org, no reseller/sub-tenant mode; zero balance stops running sandboxes after 60 min. | Isolation of arena from bot spend | None required: use two orgs, auto top-up, usage-limit breaker on the bots org. |
| G12 | Concurrency caps by tier: `inflight`, total cpu/ram (`config.ts:891-900`) and create rate limits (`rateLimit.spawn.test.ts`). | Burst of many runs | Raise org overrides; runner `MAX_CONCURRENT_RUNS` below the cap. |
| G13 | Template first boot on a host needs the image present; custom templates are not in the warm pool by default. | Cold start latency | Call `prepare`/`materialize` after each template build; keep a few bot sandboxes warm only if needed. |
| G14 | Hostname egress allowlist reads SNI/Host; with everything else denied it is unclear whether DNS is still allowed. Unverified. | Locked-down bot sandboxes | Test on first run. If DNS is blocked, allow the resolver IP explicitly. |

Arena-side changes needed (in this repo, not done): Postgres `Store`; `hosted` attestation so Platinum-run bots can be `verified`; rate limiting (or do it in Cloudflare); optional `/v1/health` alias.

## 3. Phases

1. **Phase 1, works on Platinum today:** `arena` image as a persistent sandbox (no blueprint), runner + bot template + bot sandboxes, Cloudflare Worker for `arena.sharingan.poker`. Only matches against the non-blueprint house bots.
2. **Phase 2, interim full product:** `core` image on one external big-RAM box (docker run), same Worker; everything else on Platinum. `ladder-v1` and `sharingan-1` live.
3. **Phase 3:** land G1/G2 (+G9), move `core` into the `bigmem` pool on Platinum; then G3 (custom domain) and G4 (private path) as optimizations; Postgres when scaling out.

## 4. What the user must provide

- Platinum: control-plane URL, an API key for each org (`sharingan-core` full access; `sharingan-bots` scoped `sandboxes:write`, `templates:write` for provisioning), the `apps` flag on the core org, prepaid balance or auto-top-up, region choice (same region for core and bots), and the decision on G1/G2/G9 (and a bigmem host).
- Registry: a GHCR (or other public registry) org to push `sharingan-arena|serve|core|bot:0.1.0`.
- Assets: read-only S3 credentials and URI for `sharingan-1.plcp` and `buckets-200/` (`ASSETS_S3_URI`); confirm the final blueprint file name and `ARENA_SIZING=medium`.
- DNS: `arena.sharingan.poker` on Cloudflare (Worker route, `UPSTREAM` var), and for G3 later a CNAME target from Platinum.
- Stripe: account, products/prices for credit packs, webhook endpoint and secret; refund policy for infra failures.
- Secrets: `ARENA_ADMIN_TOKEN`, `RUNNER_AUTH_TOKEN`, `SHARINGAN_API_KEYS`, billing webhook token.
- Decisions: price per 20k-hand run and `MARKUP`; Platinum's actual per-vCPU-second and per-GiB-second rates for `cost.ts`; Postgres provider.

## 5. Untested

Everything. Specifically unverified: the Docker builds (no build was run), `entrypoint.sh` `wait -n` supervision, `setpriv` inside the Platinum guest, Node `--experimental-strip-types` loading user `.ts` bots, `launch_bot.*` against a live arena, every SDK call shape in `runner/src/job.ts` (typed against the SDK source by reading, not compiled), `process.start`/`logs()` field names, `usage()` cost field names, hostname egress with DNS, and whether Platinum can pull the images. No `tsc` or `npm install` was run.
