// One eval job = one sandbox = one benchmark result. Credits are NOT touched here (service.ts reserves before and settles
// after); this module only runs the job, records what happened on the Job document, and always tears the sandbox down.
//
// Two modes (Job.input.mode):
//
//  worker   (default, scales)  the sandbox runs the MATCH WORKER: `sharingan-arena run --bot "<pipe cmd>" ...` plays the
//                              whole benchmark in-process against the house bots, the user's bot is a stdin/stdout
//                              subprocess (`sharingan pipe bot.py`, uid 10002). The sandbox has NO network. The worker signs
//                              result.json (HMAC, key written into the sandbox for the run, readable by root only); the runner
//                              reads it back and POSTs it to the arena (POST /v1/results), which checks the signature against
//                              ARENA_WORKER_KEYS and the frozen panel / seed and stores the run as verified.
//  connect  (remote-bot path)  the sandbox runs `sharingan connect` (a WebSocket client of /v1/play); the runner registers a
//                              bot identity, starts the benchmark run with the RUNNER token + hosted:true (verified) and
//                              polls progress. Slower (network round trip per decision) but needs no worker support.
//
// Steps (each names the sandbox / arena call):
//   1 provisioning  arena POST /v1/bots                               fresh bot identity (+ token, connect mode) for this job
//   2               sandbox create from the bot template, egress locked down
//   3               upload artifact (writeFile + tar), optional pip install, then drop PyPI from the allowlist
//   4 running       worker: start worker, wait for exit, read result.json, POST /v1/results
//                   connect: start launcher, POST /v1/benchmarks/{track}/runs (retry 409 not_connected), poll
//   5 finalizing    usage(), delete() in `finally`, meter COGS
import type { ArenaClient, BenchRun, ResultAck } from "./arena.ts";
import { ArenaError } from "./arena.ts";
import type { Config } from "./config.ts";
import { cogsMicro, platinumUsdFromUsage } from "./cost.ts";
import type { Logger } from "./log.ts";
import type { PlatinumClient, ProcessHandle, Sandbox } from "./platinum/types.ts";
import { sleep } from "./platinum/types.ts";
import type { ErrorClass, Job, JobStore } from "./store.ts";

export interface JobDeps {
  cfg: Config;
  platinum: PlatinumClient;
  arena: ArenaClient;
  store: JobStore;
  log: Logger;
  sleep?: (ms: number, signal?: AbortSignal) => Promise<void>;
}

/** An error with a verdict: whose fault is the failure. */
export class JobFailure extends Error {
  cls: ErrorClass;
  constructor(cls: ErrorClass, message: string) { super(message); this.cls = cls; }
}
class AbortedError extends Error {}

const PYPI_HOSTS = ["pypi.org", "files.pythonhosted.org"];
const BOT_DIR = "/opt/sharingan/bot";
const BOT_UID = 10002;
const KEY_PATH = "/opt/sharingan/worker.key";
const BOT_ENV = "HOME=/home/bot PATH=/usr/local/bin:/usr/bin:/bin PYTHONUNBUFFERED=1 PYTHONDONTWRITEBYTECODE=1";

export const shq = (s: string): string => `'${s.replace(/'/g, `'\\''`)}'`;

/** Fill `{name}` placeholders with shell-quoted values. Unknown placeholders are an error (a typo must not run). */
export function renderCommand(template: string, vars: Record<string, string | number | undefined>): string {
  return template.replace(/\{(\w+)\}/g, (_m, k: string) => {
    const v = vars[k];
    if (v === undefined) throw new JobFailure("infra", `worker command template uses {${k}}, which is not available`);
    return shq(String(v));
  });
}

export function botCommand(cfg: Config, input: Job["input"]): string {
  const pipe = input.language === "python" ? cfg.worker.pipeCmd.python : cfg.worker.pipeCmd.typescript;
  if (!pipe) throw new JobFailure("infra", `worker mode has no pipe command for ${input.language} (set PIPE_CMD_${input.language.toUpperCase()})`);
  const entry = `${BOT_DIR}/${input.entry}`;
  return `env -i ${BOT_ENV} setpriv --reuid=${BOT_UID} --regid=${BOT_UID} --clear-groups ${pipe} ${shq(entry)}`;
}

/** Best-effort progress from worker output: a JSON line carrying hands_done/hands_total, or "N/M hands". */
export function parseProgress(text: string): { done: number; total: number } | undefined {
  const lines = text.split("\n").slice(-40).reverse();
  for (const line of lines) {
    const t = line.trim();
    if (t.startsWith("{")) {
      try {
        const j = JSON.parse(t);
        const done = Number(j.hands_done ?? j.data?.hands_done);
        const total = Number(j.hands_total ?? j.data?.hands_total ?? j.hands_planned);
        if (Number.isFinite(done) && Number.isFinite(total) && total > 0) return { done, total };
      } catch { /* not json */ }
    }
    const m = t.match(/(\d+)\s*\/\s*(\d+)\s*hands/i);
    if (m) return { done: Number(m[1]), total: Number(m[2]) };
  }
  return undefined;
}

const pickNum = (o: Record<string, unknown>, keys: string[]): number | undefined => {
  for (const k of keys) if (typeof o[k] === "number" && Number.isFinite(o[k] as number)) return o[k] as number;
  return undefined;
};

export async function executeJob(deps: JobDeps, jobId: string, artifact: Uint8Array, ac: AbortController): Promise<void> {
  const { cfg, platinum, arena, store, log } = deps;
  const nap = deps.sleep ?? sleep;
  const job0 = (await store.get(jobId))!;
  const input = job0.input;
  const set = (p: Partial<Job>) => store.patch(jobId, p);
  const deadline = Date.now() + cfg.limits.runDeadlineS * 1000;
  const arenaHost = new URL(cfg.arena.botUrl).hostname;

  let sbx: Sandbox | undefined;
  let proc = undefined as ProcessHandle | undefined;   // assigned inside the mode functions
  let runStartedAt = 0;
  let verdict: { status: "finished" | "failed" | "aborted"; error?: string; cls?: ErrorClass } = { status: "finished" };
  let handsDone = 0;

  try {
    await set({ status: "provisioning" });
    const needsPip = input.language === "python" && !!input.requirementsFile;

    // 1 ---- identity: the arena attributes the run to this player (worker mode: signed into result.json via --player-id)
    const bot = await infra(() => arena.registerBot(`${input.botName}-${jobId.slice(-6)}`, input.userId));
    await set({ arenaPlayerId: bot.playerId });

    // 2 ---- sandbox. Worker mode: no egress at all. Connect mode: the arena only. PyPI only while installing.
    const baseAllow = input.mode === "connect" ? [arenaHost] : [];
    sbx = await infra(() => platinum.createSandbox({
      name: `run-${jobId}`,
      template: cfg.backend === "local" ? cfg.local.botImage : cfg.platinum.botTemplate,
      cpu: cfg.sandbox.cpu, ramMb: cfg.sandbox.ramMb, diskGb: cfg.sandbox.diskGb,
      region: cfg.platinum.region,
      autoStopMinutes: cfg.sandbox.autoStopMinutes,
      metadata: { sharingan_job: jobId, user: input.userId, track: input.track, mode: input.mode },
      allowOut: needsPip ? [...baseAllow, ...PYPI_HOSTS] : baseAllow,
    }, { signal: ac.signal, timeoutMs: 120_000 }));
    runStartedAt = Date.now();
    await set({ sandboxId: sbx.id });
    log("sandbox_created", { job: jobId, sandbox: sbx.id, backend: platinum.kind });

    // 3 ---- artifact
    if (artifact.length > cfg.limits.maxArtifactBytes) throw new JobFailure("bot", `artifact too large (${artifact.length} > ${cfg.limits.maxArtifactBytes})`);
    await infra(() => sbx!.writeFile("/tmp/bot.tgz", artifact));
    const unpack = await infra(() => sbx!.sh(
      `mkdir -p ${BOT_DIR} && tar xzf /tmp/bot.tgz -C ${BOT_DIR} --no-same-owner && chmod -R a+rX ${BOT_DIR}`, { timeoutMs: 30_000 },
    ));
    if (unpack.exitCode !== 0) throw new JobFailure("bot", `could not unpack the artifact: ${unpack.stderr.trim().slice(0, 300)}`);
    const has = await infra(() => sbx!.sh(`test -f ${shq(`${BOT_DIR}/${input.entry}`)}`));
    if (has.exitCode !== 0) throw new JobFailure("bot", `entry file ${input.entry} not found in the archive`);

    // 4 ---- deps, then drop PyPI again
    if (needsPip) {
      const pip = await infra(() => sbx!.sh(`pip install --no-cache-dir -r ${shq(`${BOT_DIR}/${input.requirementsFile!}`)}`, { timeoutMs: 300_000 }));
      if (pip.exitCode !== 0) throw new JobFailure("bot", `pip install failed: ${(pip.stderr || pip.stdout).trim().slice(-400)}`);
      await infra(() => sbx!.setEgress(baseAllow));
    }

    // 5 ---- play
    await set({ status: input.mode === "worker" ? "running" : "connecting" });
    if (input.mode === "worker") {
      const r = await runWorker(bot);
      handsDone = r.handsDone;
    } else {
      const r = await runConnect(bot);
      handsDone = r.handsDone;
    }
  } catch (e) {
    if (e instanceof AbortedError) verdict = { status: "aborted", error: "aborted" };
    else if (e instanceof JobFailure) verdict = { status: "failed", error: e.message.slice(0, 1000), cls: e.cls };
    else verdict = { status: "failed", error: String((e as Error)?.message ?? e).slice(0, 1000), cls: "infra" };
    log("job_error", { job: jobId, status: verdict.status, cls: verdict.cls, error: verdict.error });
  } finally {
    // 6 ---- meter + teardown. Always runs. usage() must be read BEFORE delete (404 afterwards).
    const patch: Partial<Job> = {};
    if (proc) {
      try { const l = await proc.logs(); patch.botLogTail = `${l.stderr}${l.stdout ? `\n--- stdout ---\n${l.stdout}` : ""}`.slice(-4000); } catch { /* best effort */ }
      try { if ((await proc.status()).running) await proc.kill(); } catch { /* the sandbox is about to be deleted */ }
    }
    let platinumUsd: number | undefined;
    if (sbx) {
      try { platinumUsd = platinumUsdFromUsage(await sbx.usage()); } catch { /* best effort */ }
      try { await sbx.delete(); patch.sandboxDeleted = true; } catch (e) {
        patch.sandboxDeleted = false;
        log("sandbox_leak", { job: jobId, sandbox: sbx.id, error: String(e) });   // the reaper retries
      }
    }
    const seconds = runStartedAt ? (Date.now() - runStartedAt) / 1000 : 0;
    const j = (await store.get(jobId))!;
    const done = Math.max(handsDone, j.handsDone ?? 0);
    await set({
      ...patch,
      status: verdict.status,
      ...(verdict.error ? { error: verdict.error, errorClass: verdict.cls } : {}),
      handsDone: done,
      sandboxSeconds: seconds,
      platinumUsd,
      cogsMicro: cogsMicro(cfg.pricing, { cpu: cfg.sandbox.cpu, ramMb: cfg.sandbox.ramMb, seconds, handsPlayed: done, platinumUsd }),
      finishedAt: Date.now(),
    });
  }

  // ------------------------------------------------------------------------------------------ worker mode
  async function runWorker(bot: { playerId: string }): Promise<{ handsDone: number }> {
    const w = cfg.worker;
    if (!w.signKey) throw new JobFailure("infra", "worker mode needs WORKER_SIGN_KEY (the worker signs result.json with it)");
    // --sign-key-file takes `id=secret` (matching an entry of the arena's ARENA_WORKER_KEYS) or a bare secret
    const keyFile = w.keyId ? `${w.keyId}=${w.signKey}` : w.signKey;
    await infra(() => sbx!.writeFile(KEY_PATH, new TextEncoder().encode(keyFile), { mode: 0o400 }));
    const dir = w.resultPath.slice(0, w.resultPath.lastIndexOf("/")) || "/";
    const cmd = renderCommand(w.cmdTemplate, {
      bot: botCommand(cfg, input),
      track: input.track,
      hands: input.hands,
      lanes: input.lanes ?? cfg.limits.defaultLanes,
      out: w.resultPath,
      keyfile: KEY_PATH,
      player_id: bot.playerId,
      job_id: jobId,
      name: input.botName,
      owner: input.userId,
      key_id: w.keyId,
    });
    const wp = await infra(() => sbx!.startProcess({
      cmd: ["sh", "-c", `mkdir -p ${shq(dir)} && cd ${shq(dir)} && exec ${cmd}`],
      env: { ...w.env, SHARINGAN_JOB_ID: jobId, SHARINGAN_OWNER: input.userId, SHARINGAN_BOT_NAME: input.botName, SHARINGAN_TRACK: input.track },
      timeoutMs: Math.max(1000, deadline - Date.now()),
    }));
    proc = wp;
    await set({ status: "running", handsTotal: input.hands });

    // wait for the worker to exit
    let failures = 0;
    let exitCode: number | undefined;
    for (;;) {
      if (ac.signal.aborted) throw new AbortedError();
      if (Date.now() > deadline) throw new JobFailure("bot", `run deadline (${cfg.limits.runDeadlineS}s) exceeded`);
      try {
        const st = await wp.status();
        failures = 0;
        if (!st.running) { exitCode = st.exitCode ?? 0; break; }
        const l = await wp.logs();
        const p = parseProgress(`${l.stderr}\n${l.stdout}`);
        if (p) await set({ handsDone: Math.min(p.done, input.hands), handsTotal: p.total });
      } catch (e) {
        if (++failures >= cfg.limits.pollFailureLimit) throw new JobFailure("infra", `sandbox unreachable: ${String(e).slice(0, 200)}`);
      }
      await nap(cfg.limits.pollIntervalMs, ac.signal).catch(() => { throw new AbortedError(); });
    }
    if (exitCode !== 0) {
      const l = await wp.logs().catch(() => ({ stdout: "", stderr: "" }));
      throw new JobFailure("bot", `worker exited with code ${exitCode}: ${(l.stderr || l.stdout).trim().slice(-500)}`);
    }

    // result.json -> arena
    await set({ status: "finalizing" });
    const raw = await infra(() => sbx!.readFile(w.resultPath));
    let result: Record<string, unknown>;
    try { result = JSON.parse(new TextDecoder().decode(raw)); } catch { throw new JobFailure("infra", "worker wrote an unreadable result.json"); }
    // A bot that never answered a single decision (import error, crash on startup) forfeits every hand. The worker still
    // produces a valid signed result, but posting it would put a junk run on the leaderboard: fail the job instead.
    const forfeited = pickNum(result, ["forfeit_hands"]) ?? 0;
    const played = pickNum(result, ["hands", "hands_done"]) ?? 0;
    if (played > 0 && forfeited >= played) {
      const l = await wp.logs().catch(() => ({ stdout: "", stderr: "" }));
      await set({ handsDone: 0, forfeitHands: forfeited });
      throw new JobFailure("bot", `the bot never answered a decision (${forfeited}/${played} hands forfeited); nothing was submitted. Last output: ${(l.stderr || l.stdout).trim().slice(-400)}`);
    }
    let ack: ResultAck;
    try {
      ack = await arena.postResult(raw);
    } catch (e) {
      if (e instanceof ArenaError && e.status >= 400 && e.status < 500) throw new JobFailure("infra", `arena rejected the signed result: ${e.message}`);
      throw new JobFailure("infra", `could not submit the result to the arena: ${String(e).slice(0, 200)}`);
    }
    const handsPlayed = pickNum(ack, ["hands_done", "hands"]) ?? pickNum(result, ["hands_done", "hands_played", "hands"]) ?? 0;
    await set({
      handsDone: handsPlayed,
      handsTotal: pickNum(ack, ["hands_total", "hands_planned"]) ?? pickNum(result, ["hands_total", "hands_planned"]) ?? input.hands,
      runId: typeof ack.run_id === "string" ? ack.run_id : typeof result.run_id === "string" ? result.run_id : undefined,
      mbbPerGame: ack.mbb_per_game ?? pickNum(result, ["mbb_per_game"]) ?? null,
      ci95Mbb: ack.ci95_mbb ?? pickNum(result, ["ci95_mbb"]) ?? null,
      verified: ack.verified === true,
      forfeitHands: pickNum(ack, ["forfeit_hands"]) ?? pickNum(result, ["forfeit_hands"]),
      resultAck: ack,
    });
    if (ack.verified !== true) throw new JobFailure("infra", "arena accepted the result but did not mark it verified (check ARENA_WORKER_KEYS / WORKER_KEY_ID)");
    return { handsDone: handsPlayed };
  }

  // ------------------------------------------------------------------------------------------ connect mode
  async function runConnect(bot: { playerId: string; token: string }): Promise<{ handsDone: number }> {
    // The token goes in the process env only, never in the sandbox's create-time env.
    const launcher = input.language === "python" ? ["python3", "/opt/sharingan/launch_bot.py"] : ["node", "/opt/sharingan/launch_bot.mjs"];
    const cp = await infra(() => sbx!.startProcess({
      cmd: ["setpriv", `--reuid=${BOT_UID}`, `--regid=${BOT_UID}`, "--clear-groups", ...launcher],
      cwd: BOT_DIR,
      env: { BOT_FILE: `${BOT_DIR}/${input.entry}`, ARENA_URL: cfg.arena.botUrl, ARENA_TOKEN: bot.token, PYTHONUNBUFFERED: "1", HOME: "/home/bot" },
      timeoutMs: Math.max(1000, deadline - Date.now()),   // guest-side hard stop for the bot process
    }));
    proc = cp;

    // start the run once the bot's websocket is up (409 not_connected until then)
    let run: BenchRun | undefined;
    const connectBy = Date.now() + cfg.limits.connectTimeoutS * 1000;
    while (!run) {
      if (ac.signal.aborted) throw new AbortedError();
      try {
        run = await arena.startHostedRun(input.track, bot.playerId, { hands: input.hands, lanes: input.lanes ?? cfg.limits.defaultLanes });
      } catch (e) {
        if (!(e instanceof ArenaError) || e.code !== "not_connected") {
          if (e instanceof ArenaError && e.status >= 400 && e.status < 500) throw new JobFailure("infra", `arena refused the run: ${e.message}`);
          throw new JobFailure("infra", String(e).slice(0, 300));
        }
        if ((await cp.status().catch(() => ({ running: true }))).running === false) {
          const l = await cp.logs().catch(() => ({ stdout: "", stderr: "" }));
          throw new JobFailure("bot", `bot exited before connecting: ${(l.stderr || l.stdout).trim().slice(-500)}`);
        }
        if (Date.now() > connectBy) {
          const l = await cp.logs().catch(() => ({ stdout: "", stderr: "" }));
          throw new JobFailure("bot", `bot did not connect in ${cfg.limits.connectTimeoutS}s: ${(l.stderr || l.stdout).trim().slice(-500)}`);
        }
        await nap(300, ac.signal).catch(() => { throw new AbortedError(); });
      }
    }
    await set({ status: "running", runId: run.run_id, handsTotal: run.hands_planned ?? run.hands_total ?? input.hands });

    // poll progress
    let failures = 0;
    while (run.status === "running") {
      if (ac.signal.aborted || Date.now() > deadline) {
        await arena.abortRun(input.track, run.run_id).catch(() => {});
        if (ac.signal.aborted) throw new AbortedError();
        throw new JobFailure("bot", `run deadline (${cfg.limits.runDeadlineS}s) exceeded`);
      }
      await nap(cfg.limits.pollIntervalMs, ac.signal).catch(() => { throw new AbortedError(); });
      try {
        run = await arena.getRun(input.track, run.run_id);
        failures = 0;
      } catch (e) {
        if (++failures >= cfg.limits.pollFailureLimit) throw new JobFailure("infra", `arena unreachable while polling: ${String(e).slice(0, 200)}`);
        continue;
      }
      await set({ handsDone: run.hands_done, runningMbb: run.running_mbb_per_game ?? null });
      if (run.status === "running" && (await cp.status().catch(() => ({ running: true }))).running === false) {
        await arena.abortRun(input.track, run.run_id).catch(() => {});
        const l = await cp.logs().catch(() => ({ stdout: "", stderr: "" }));
        handsDone = run.hands_done;
        await set({ handsDone });
        throw new JobFailure("bot", `bot process exited mid-run: ${(l.stderr || l.stdout).trim().slice(-500)}`);
      }
    }
    await set({
      status: "finalizing", handsDone: run.hands_done, handsTotal: run.hands_planned ?? run.hands_total,
      mbbPerGame: run.mbb_per_game, ci95Mbb: run.ci95_mbb, verified: run.verified,
    });
    if (run.status !== "finished") throw new JobFailure("bot", `run ended as ${run.status}`);
    return { handsDone: run.hands_done };
  }
}

/** Anything thrown by `fn` is our infrastructure's fault unless it already carries a verdict. */
async function infra<T>(fn: () => Promise<T>): Promise<T> {
  try { return await fn(); } catch (e) {
    if (e instanceof JobFailure || e instanceof AbortedError) throw e;
    if (e instanceof Error && e.message === "aborted") throw new AbortedError();
    throw new JobFailure("infra", String((e as Error)?.message ?? e).slice(0, 500));
  }
}

/** Delete run sandboxes that no active job owns (runner crash, lost delete). Call on boot and every few minutes. */
export async function reapOrphans(deps: Pick<JobDeps, "platinum" | "store" | "log">, minAgeMs = 120_000): Promise<number> {
  const owned = new Set((await deps.store.active()).map((j) => j.sandboxId).filter(Boolean));
  let n = 0;
  for await (const s of deps.platinum.listSandboxes()) {
    if (!s.metadata.sharingan_job || owned.has(s.id)) continue;
    if (Date.now() - s.createdAtMs < minAgeMs) continue;     // the create may not be recorded in the store yet
    try { await deps.platinum.deleteSandbox(s.id); n++; deps.log("reaped_sandbox", { sandbox: s.id, job: s.metadata.sharingan_job }); } catch { /* next sweep */ }
  }
  return n;
}
