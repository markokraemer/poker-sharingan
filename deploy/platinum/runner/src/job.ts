// One benchmark job = one sandbox = one arena run. UNTESTED against live Platinum or a live arena.
//
// Flow (each step names the Platinum call it uses; DESIGN.md section 5 has the repo paths):
//   1 arena  POST /v1/bots                          fresh bot identity + token for this job
//   2 pt     sandboxes.create({template,...})       isolated microVM, egress locked to the arena host
//   3 pt     sbx.files.write + sbx.sh(tar)          upload the user's artifact
//   4 pt     sbx.sh(pip install) + updateNetwork    optional deps, then drop PyPI from the allowlist
//   5 pt     sbx.process.start(launch_bot.*)        bot connects out to wss://arena/v1/play
//   6 arena  POST /v1/benchmarks/{track}/runs       retried while 409 not_connected
//   7 arena  GET  .../runs/{id}                     poll progress until finished / aborted
//   8 pt     sbx.usage(); sbx.delete()              meter, then tear down (always, in finally)
import { Platinum, type Sandbox } from "@platinum-dev/sdk";
import { config } from "./config.ts";
import * as arena from "./arena.ts";
import { chargeUsd, platinumUsdFromUsage } from "./cost.ts";
import { store, type Job, type JobInput } from "./store.ts";

export const pt = new Platinum({ url: config.ptApiUrl, token: config.ptToken });
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const aborts = new Map<string, AbortController>();

export function requestAbort(jobId: string) { aborts.get(jobId)?.abort(); }

const PYPI_HOSTS = ["pypi.org", "files.pythonhosted.org"];

export async function runJob(jobId: string, input: JobInput): Promise<void> {
  const ac = new AbortController();
  aborts.set(jobId, ac);
  const t0 = Date.now();
  const deadline = t0 + config.runDeadlineS * 1000;
  let sbx: Sandbox | undefined;
  let botToken = "";
  let runStartedAt = 0;
  const set = (p: Partial<Job>) => store.patch(jobId, p);

  try {
    // 1 ---- bot identity
    set({ status: "provisioning" });
    const bot = await arena.registerBot(`${input.botName}-${jobId.slice(-6)}`, input.userId);
    botToken = bot.token;
    set({ arenaPlayerId: bot.playerId });

    // 2 ---- sandbox. Egress: arena host only (+ PyPI during setup if the bot has requirements).
    const arenaHost = new URL(config.arenaUrl).hostname;
    const needsPip = input.language === "python" && !!input.requirementsFile;
    sbx = await pt.sandboxes.create(
      {
        name: `run-${jobId}`,
        template: config.botTemplate,
        type: "ephemeral",
        auto_stop_minutes: 30,                 // belt and braces; the runner deletes explicitly
        cpu: config.cpu, ram_mb: config.ramMb, disk_gb: config.diskGb,
        region: config.region,
        metadata: { sharingan_job: jobId, user: input.userId, track: input.track },
        network: {
          allowOut: needsPip ? [arenaHost, ...PYPI_HOSTS] : [arenaHost],
          denyOut: ({ allTraffic }) => [allTraffic],
        },
      },
      { waitForRunning: true, waitTimeoutMs: 120_000, signal: ac.signal },
    );
    runStartedAt = Date.now();
    set({ sandboxId: sbx.id });

    // 3 ---- artifact
    const bytes = Buffer.from(input.artifactTgzBase64, "base64");
    if (bytes.length > config.maxArtifactBytes) throw new Error(`artifact too large (${bytes.length} > ${config.maxArtifactBytes})`);
    await sbx.files.write("/tmp/bot.tgz", bytes);
    (await sbx.sh(
      "mkdir -p /opt/sharingan/bot && tar xzf /tmp/bot.tgz -C /opt/sharingan/bot --no-same-owner && chmod -R a+rX /opt/sharingan/bot",
      { timeoutMs: 30_000 },
    )).check();

    // 4 ---- deps, then lock egress down to the arena only
    if (needsPip) {
      (await sbx.sh(`pip install --no-cache-dir -r /opt/sharingan/bot/${shq(input.requirementsFile!)}`, { timeoutMs: 300_000 })).check();
      await sbx.updateNetwork({ allowOut: [arenaHost], denyOut: ({ allTraffic }) => [allTraffic] });
    }

    // 5 ---- start the bot (connect mode). Token goes in the process env only, never in the sandbox's create-time env.
    set({ status: "connecting" });
    const entryPath = `/opt/sharingan/bot/${input.entry}`;
    const launcher = input.language === "python"
      ? ["python3", "/opt/sharingan/launch_bot.py"]
      : ["node", "/opt/sharingan/launch_bot.mjs"];
    const proc = await sbx.process.start({
      cmd: ["setpriv", "--reuid=10002", "--regid=10002", "--clear-groups", ...launcher],
      cwd: "/opt/sharingan/bot",
      env: { BOT_FILE: entryPath, ARENA_URL: config.arenaUrl, ARENA_TOKEN: botToken, PYTHONUNBUFFERED: "1", HOME: "/home/bot" },
      timeoutMs: Math.max(1000, deadline - Date.now()),   // guest-side hard stop for the bot process
    });

    // 6 ---- start the run once the bot's websocket is up
    let run: arena.BenchRun | undefined;
    const connectBy = Date.now() + config.connectTimeoutS * 1000;
    while (!run) {
      if (ac.signal.aborted) throw new AbortedError();
      try {
        run = await arena.startRun(input.track, botToken, { hands: input.hands, lanes: input.lanes });
      } catch (e) {
        const notYet = e instanceof arena.ArenaError && e.code === "not_connected";
        if (!notYet) throw e;
        if (Date.now() > connectBy) throw new Error(`bot did not connect in ${config.connectTimeoutS}s: ${await tail(proc)}`);
        await sleep(500);
      }
    }
    set({ status: "running", runId: run.run_id, handsTotal: run.hands_planned });

    // 7 ---- poll
    while (run.status === "running") {
      if (ac.signal.aborted || Date.now() > deadline) {
        await arena.abortRun(input.track, run.run_id, botToken).catch(() => {});
        if (ac.signal.aborted) throw new AbortedError();
        throw new Error("run deadline exceeded");
      }
      await sleep(config.pollIntervalMs);
      run = await arena.getRun(input.track, run.run_id);
      set({ handsDone: run.hands_done, runningMbb: run.running_mbb_per_game ?? null });
    }
    set({
      status: "finalizing", handsDone: run.hands_done,
      mbbPerGame: run.mbb_per_game, ci95Mbb: run.ci95_mbb, verified: run.verified,
    });
  } catch (e) {
    const aborted = e instanceof AbortedError;
    set({ status: aborted ? "aborted" : "failed", error: aborted ? "aborted" : String((e as Error)?.message ?? e).slice(0, 1000) });
  } finally {
    // 8 ---- meter + teardown. Always runs. usage() must be read BEFORE delete (404 afterwards).
    const j = store.get(jobId)!;
    let platinumUsd: number | undefined;
    if (sbx) {
      try { platinumUsd = platinumUsdFromUsage(await sbx.usage()); } catch { /* best effort */ }
      try { await sbx.delete(); } catch (e) { console.error(`LEAK? failed to delete ${sbx.id}`, e); }
    }
    const seconds = runStartedAt ? (Date.now() - runStartedAt) / 1000 : 0;
    const charge = chargeUsd({ cpu: config.cpu, ramMb: config.ramMb, seconds, handsPlayed: j.handsDone ?? 0, platinumUsd });
    // A failed run caused by our infra should not be billed: the billing service decides from status + error.
    store.patch(jobId, {
      sandboxSeconds: seconds, platinumUsd, chargeUsd: charge,
      status: j.status === "finalizing" ? "finished" : j.status,
    });
    aborts.delete(jobId);
    void notifyBilling(jobId);
  }
}

class AbortedError extends Error {}

const shq = (s: string) => `'${s.replace(/'/g, `'\\''`)}'`;

async function tail(proc: { logs(): Promise<{ stdout_text?: string; stderr_text?: string }> }): Promise<string> {
  try { const l = await proc.logs(); return `${l.stderr_text ?? ""}${l.stdout_text ?? ""}`.slice(-600); } catch { return "(no logs)"; }
}

/** Tell the billing service the job settled (it owns credits; this service only reports usage). Idempotent by idempotencyKey. */
async function notifyBilling(jobId: string) {
  const url = process.env.BILLING_WEBHOOK_URL;
  if (!url) return;
  const j = store.get(jobId)!;
  try {
    await fetch(url, {
      method: "POST",
      headers: { "content-type": "application/json", authorization: `Bearer ${process.env.BILLING_WEBHOOK_TOKEN ?? ""}` },
      body: JSON.stringify({ idempotencyKey: j.input.idempotencyKey, jobId, status: j.status, chargeUsd: j.chargeUsd, platinumUsd: j.platinumUsd, sandboxSeconds: j.sandboxSeconds, handsDone: j.handsDone }),
    });
  } catch (e) { console.error("billing webhook failed (will be re-read via GET /jobs/:id)", e); }
}

/** Delete run sandboxes that no active job owns (runner crash, lost delete). Call on boot and every few minutes. */
export async function reapOrphans(): Promise<number> {
  const owned = new Set(store.active().map((j) => j.sandboxId).filter(Boolean));
  let n = 0;
  for await (const s of pt.sandboxes.iterate({ state: "running" })) {
    const meta = (s as any).metadata as Record<string, unknown> | undefined;
    if (!meta?.sharingan_job || owned.has((s as any).id)) continue;
    const age = Date.now() - new Date((s as any).createdAt ?? (s as any).created_at ?? 0).getTime();
    if (age < 120_000) continue;                     // create may not be recorded in the store yet
    try { await (await pt.sandboxes.connect((s as any).id)).delete(); n++; } catch { /* next sweep */ }
  }
  return n;
}
