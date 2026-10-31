// executeJob against a fake sandbox backend and a fake arena: both modes, every failure verdict, teardown always.
import assert from "node:assert/strict";
import { afterEach, beforeEach, describe, test } from "node:test";
import { ArenaClient } from "../src/arena.ts";
import type { Config } from "../src/config.ts";
import { botCommand, executeJob, parseProgress, reapOrphans, renderCommand, SEED_PATH, shq, type JobDeps } from "../src/job.ts";
import { noopLog } from "../src/log.ts";
import { MemoryJobStore, type Job, type JobInput } from "../src/store.ts";
import { FakeArena, FakePlatinum, until, workerResult, type Behavior } from "./helpers.ts";
import { testConfig } from "./helpers.ts";

let arena: FakeArena;
let cfg: Config;

beforeEach(async () => { arena = new FakeArena(); await arena.start(); cfg = testConfig(arena.url); });
afterEach(async () => { await arena.stop(); });

const input = (over: Partial<JobInput> = {}): JobInput => ({
  userId: "user_a", botName: "mybot", language: "python", entry: "bot.py", track: "ladder-v1", hands: 600, idempotencyKey: "k1", mode: "worker", ...over,
});

async function setup(behavior: Behavior, over: Partial<JobInput> = {}, cfgOver: Record<string, string> = {}) {
  if (Object.keys(cfgOver).length) cfg = testConfig(arena.url, cfgOver);
  const platinum = new FakePlatinum(behavior);
  const store = new MemoryJobStore();
  const job: Job = { id: "job_t1", input: input(over), status: "queued", createdAt: Date.now(), reservedMicro: 1, settled: false };
  await store.put(job);
  const deps: JobDeps = { cfg, platinum, arena: new ArenaClient(cfg.arena), store, log: noopLog };
  const ac = new AbortController();
  const run = () => executeJob(deps, job.id, new Uint8Array([0x1f, 0x8b, 1, 2, 3]), ac);
  return { platinum, store, run, ac, deps };
}

/** The "worker": writes result.json and exits 0 shortly after being started. */
const goodWorker = (result = workerResult()): Behavior["onProcess"] => (p, sbx) => {
  p.stderr = '{"hands_done":300,"hands_total":600}\n';
  setTimeout(() => { sbx.files.set("/work/result.json", new TextEncoder().encode(result)); p.exit(0); }, 30);
};

describe("worker mode", () => {
  test("happy path: sandbox without network, key mode 0400, bot as uid 10002, result POSTed to the arena, sandbox deleted", async () => {
    const { platinum, store, run } = await setup({ onProcess: goodWorker(), usage: { cost_usd: 0.002 } });
    await run();
    const j = (await store.get("job_t1"))!;
    assert.equal(j.status, "finished");
    assert.equal(j.verified, true);
    assert.equal(j.handsDone, 600);
    assert.equal(j.mbbPerGame, 12.5);
    assert.equal(j.runId, "r_worker");
    assert.equal(j.sandboxDeleted, true);
    assert.equal(j.platinumUsd, 0.002);
    assert.ok(j.cogsMicro! > 0);
    assert.ok(j.finishedAt);

    const sbx = platinum.last;
    assert.equal(sbx.deleted, true);
    assert.deepEqual(sbx.spec.allowOut, [], "worker sandboxes have no network at all");
    assert.equal(sbx.spec.metadata.sharingan_job, "job_t1");
    assert.ok(sbx.files.has("/tmp/bot.tgz"));
    assert.equal(sbx.fileModes.get("/opt/sharingan/worker.key"), 0o400);
    assert.equal(new TextDecoder().decode(sbx.files.get("/opt/sharingan/worker.key")), "key1=test-signing-key", "id=secret, as --sign-key-file expects");
    const cmd = sbx.procs[0]!.opts.cmd.join(" ");
    assert.match(cmd, /sharingan-arena run --bot 'env -i HOME=\/tmp .*PYTHONPATH=\/opt\/sharingan\/deps .* sharingan pipe '\\''\/opt\/sharingan\/bot\/bot\.py'\\'''/);
    assert.match(cmd, / --rotation-uids 10010,10011,10012,10013,10014,10015$/, "every signed run is played with each rotation as its own user");
    const unpack = sbx.shCalls.find((s) => s.includes("tar xzf"))!;
    assert.match(unpack, /setpriv --reuid=10002 .* tar xzf \/tmp\/bot\.tgz -C \/opt\/sharingan\/bot/, "unpacked by the unprivileged user");
    assert.ok(unpack.includes("chown -R root:root /opt/sharingan/bot") && unpack.includes("go-w"), "then the bot can't write its own directory");
    assert.match(unpack, /find -P \/opt\/sharingan\/bot -exec touch -a -h -d 2099-01-01 \{\} \+$/, "and reading a file leaves no access-time mark");
    assert.match(cmd, /--track 'ladder-v1' --hands '600' --lanes '1' --out '\/work\/result\.json' --sign-key-file '\/opt\/sharingan\/worker\.key' --player-id 'p_test'/);
    assert.equal(sbx.procs[0]!.opts.env!.SHARINGAN_JOB_ID, "job_t1");
    assert.ok(!JSON.stringify(sbx.procs[0]!.opts).includes("runner-secret"), "no arena token reaches the sandbox");

    const posted = arena.called("POST", "/v1/results");
    assert.equal(posted.length, 1);
    assert.equal(posted[0]!.auth, "Bearer runner-secret");
    assert.equal(posted[0]!.body.signature.value, "deadbeef", "result.json is forwarded verbatim");
    assert.equal(arena.called("POST", "/v1/bots").length, 1, "one bot identity per job, signed into the result via --player-id");
    assert.equal(arena.called("POST", "/v1/bots")[0]!.body.owner, "user_a");
  });

  test("fresh secret deals: run:<64 hex> seed, root-only file, passed as --seed-file, never in env/args, shown only after the job", async () => {
    let midRun: Job | undefined;
    let st: MemoryJobStore | undefined;
    const { platinum, store, run } = await setup({
      onProcess: (p, sbx) => {
        p.stderr = '{"hands_done":300,"hands_total":600}\n';
        setTimeout(async () => { midRun = await st!.get("job_t1"); sbx.files.set("/work/result.json", new TextEncoder().encode(workerResult())); p.exit(0); }, 30);
      },
    });
    st = store;
    await run();
    const sbx = platinum.last;
    const seed = new TextDecoder().decode(sbx.files.get(SEED_PATH));
    assert.equal(SEED_PATH, "/opt/sharingan/run.seed");
    assert.match(seed, /^run:[0-9a-f]{64}$/);
    assert.equal(sbx.fileModes.get(SEED_PATH), 0o400, "readable by root only, like the signing key");
    assert.equal(sbx.fileModes.get("/opt/sharingan/worker.key"), 0o400);
    const proc = sbx.procs[0]!;
    assert.match(proc.opts.cmd.join(" "), /--seed-file '\/opt\/sharingan\/run\.seed'/);
    assert.ok(!JSON.stringify(proc.opts).includes(seed), "the seed itself is in nobody's env or argv");
    assert.ok(!JSON.stringify(sbx.spec).includes(seed));
    assert.ok(!botCommand(cfg, input()).includes(seed));
    assert.ok(!sbx.shCalls.join("\n").includes(seed), "not in any shell command either");
    assert.equal(midRun?.dealsSeed, undefined, "not recorded while the job is running");
    assert.equal((await store.get("job_t1"))!.dealsSeed, seed, "recorded for audit once the job is over");
  });

  test("every job gets its own seed", async () => {
    const seeds: string[] = [];
    for (let i = 0; i < 2; i++) {
      const { platinum, run } = await setup({ onProcess: goodWorker() });
      await run();
      seeds.push(new TextDecoder().decode(platinum.last.files.get(SEED_PATH)));
    }
    assert.notEqual(seeds[0], seeds[1]);
  });

  test("a custom WORKER_CMD_TEMPLATE without {seedfile} still gets --seed-file appended", async () => {
    const { platinum, run } = await setup({ onProcess: goodWorker() }, {}, {
      WORKER_CMD_TEMPLATE: "sharingan-arena run --bot {bot} --track {track} --hands {hands} --out {out} --sign-key-file {keyfile} --player-id {player_id}",
    });
    await run();
    assert.match(platinum.last.procs[0]!.opts.cmd.join(" "), /--player-id 'p_test' --seed-file '\/opt\/sharingan\/run\.seed' --rotation-uids 10010,10011,10012,10013,10014,10015$/);
  });

  test("a failing bot: one clear line, ONE de-duplicated traceback, no progress, no score line (error and bot_log_tail)", async () => {
    const tb = "Traceback (most recent call last):\n  File \"/opt/sharingan/bot/bot.py\", line 3, in <module>\n    import nope\nModuleNotFoundError: No module named 'nope'\n";
    const stderr = ["lane 0", "lane 1", "lane 2", "lane 3"].map((l, i) => `${tb}  ladder-v1 ${(i + 1) * 100}/600 hands (50 hands/s)\n`).join("")
      + '{"hands_done":600,"hands_total":600}\nladder-v1: +0.0 \u00b1 0.0 mbb/game (aivat) over 600 hands on ladder-v1; 50 hands/s; 600 FORFEIT hands; signed\n';
    const { store, run } = await setup({
      onProcess: (p, sbx) => { p.stderr = stderr; setTimeout(() => { sbx.files.set("/work/result.json", new TextEncoder().encode(workerResult({ forfeit_hands: 600 }))); p.exit(0); }, 20); },
    });
    await run();
    const j = (await store.get("job_t1"))!;
    assert.equal(j.status, "failed");
    const [first, ...rest] = j.error!.split("\n");
    assert.equal(first, "the bot never answered a decision (600/600 hands forfeited); nothing was submitted.");
    const body = rest.join("\n");
    assert.equal(body.split("Traceback").length - 1, 1, body);
    assert.equal(body.split("ModuleNotFoundError").length - 1, 1);
    for (const t of [body, j.botLogTail!]) {
      assert.doesNotMatch(t, /mbb\/game|FORFEIT|signed|hands_done|\d+\/\d+ hands/);
    }
    assert.doesNotMatch(j.error!, /mbb\/game|FORFEIT|signed/);
    assert.equal(j.botLogTail!.split("Traceback").length - 1, 1, j.botLogTail);
    assert.match(j.botLogTail!, /ModuleNotFoundError/);
  });

  test("a finished run keeps its closing summary line in bot_log_tail", async () => {
    const { store, run } = await setup({
      onProcess: (p, sbx) => { p.stderr = "ladder-v1: +12.5 \u00b1 3.2 mbb/game (aivat) over 600 hands on ladder-v1; 50 hands/s; signed\n"; setTimeout(() => { sbx.files.set("/work/result.json", new TextEncoder().encode(workerResult())); p.exit(0); }, 20); },
    });
    await run();
    const j = (await store.get("job_t1"))!;
    assert.equal(j.status, "finished");
    assert.match(j.botLogTail!, /mbb\/game/);
  });

  test("progress lines in the worker's output update the job while it runs", async () => {
    const { store, run } = await setup({
      onProcess: (p, sbx) => {
        p.stderr = '{"hands_done":300,"hands_total":600}\n';
        setTimeout(() => { sbx.files.set("/work/result.json", new TextEncoder().encode(workerResult())); p.exit(0); }, 120);
      },
    });
    const seen: number[] = [];
    const t = setInterval(async () => { const h = (await store.get("job_t1"))?.handsDone; if (h) seen.push(h); }, 10);
    await run();
    clearInterval(t);
    assert.ok(seen.includes(300), `saw ${JSON.stringify(seen)}`);
  });

  test("the worker failing is the bot's problem: failed/bot, log tail kept, sandbox deleted", async () => {
    const { platinum, store, run } = await setup({ onProcess: (p) => setTimeout(() => p.exit(3, "Traceback: bot.py line 3: boom\n"), 20) });
    await run();
    const j = (await store.get("job_t1"))!;
    assert.deepEqual([j.status, j.errorClass], ["failed", "bot"]);
    assert.match(j.error!, /worker exited with code 3.*boom/s);
    assert.match(j.botLogTail!, /boom/);
    assert.equal(platinum.last.deleted, true);
    assert.equal(arena.called("POST", "/v1/results").length, 0);
  });

  test("a bot that never answered a decision (all hands forfeited) fails as bot and is NOT posted to the leaderboard", async () => {
    const { store, run, platinum } = await setup({ onProcess: goodWorker(workerResult({ forfeit_hands: 600 })) });
    await run();
    const j = (await store.get("job_t1"))!;
    assert.deepEqual([j.status, j.errorClass, j.handsDone, j.forfeitHands], ["failed", "bot", 0, 600]);
    assert.match(j.error!, /never answered a decision/);
    assert.equal(arena.called("POST", "/v1/results").length, 0);
    assert.equal(platinum.last.deleted, true);
  });

  test("some forfeited hands (bot crashed mid-run) are still a valid, billed result, flagged forfeit_hands", async () => {
    const { store, run } = await setup({ onProcess: goodWorker(workerResult({ forfeit_hands: 120 })) });
    await run();
    const j = (await store.get("job_t1"))!;
    assert.deepEqual([j.status, j.verified, j.forfeitHands, j.handsDone], ["finished", true, 120, 600]);
  });

  test("the arena rejecting the signed result is OUR problem (infra), not the user's", async () => {
    arena.resultStatus = 400;
    const { platinum, store, run } = await setup({ onProcess: goodWorker() });
    await run();
    const j = (await store.get("job_t1"))!;
    assert.deepEqual([j.status, j.errorClass], ["failed", "infra"]);
    assert.match(j.error!, /rejected the signed result.*signature mismatch/);
    assert.equal(platinum.last.deleted, true);
  });

  test("an accepted but unverified result is not a success", async () => {
    arena.resultVerified = false;
    const { store, run } = await setup({ onProcess: goodWorker() });
    await run();
    const j = (await store.get("job_t1"))!;
    assert.deepEqual([j.status, j.errorClass], ["failed", "infra"]);
    assert.match(j.error!, /not mark it verified/);
  });

  test("missing signing key is a configuration error before any sandbox work", async () => {
    const { store, run, platinum } = await setup({ onProcess: goodWorker() }, {}, { WORKER_SIGN_KEY: "" });
    await run();
    const j = (await store.get("job_t1"))!;
    assert.equal(j.status, "failed");
    assert.match(j.error!, /WORKER_SIGN_KEY/);
    assert.equal(platinum.last.deleted, true);
  });

  test("an unreadable result.json is infra", async () => {
    const { store, run } = await setup({ onProcess: goodWorker("not json {") });
    await run();
    assert.match((await store.get("job_t1"))!.error!, /unreadable result.json/);
  });

  test("deadline: a worker that never ends is killed and the job fails as bot, with the sandbox deleted", async () => {
    const { platinum, store, run } = await setup({ onProcess: () => {} }, {}, { RUN_DEADLINE_S: "0.15" });
    await run();
    const j = (await store.get("job_t1"))!;
    assert.deepEqual([j.status, j.errorClass], ["failed", "bot"]);
    assert.match(j.error!, /deadline/);
    assert.equal(platinum.last.procs[0]!.killed, true);
    assert.equal(platinum.last.deleted, true);
  });

  test("abort while the worker runs -> aborted, sandbox deleted", async () => {
    const { platinum, store, run, ac } = await setup({ onProcess: () => {} });
    const p = run();
    await until(async () => (await store.get("job_t1"))?.status === "running");
    ac.abort();
    await p;
    const j = (await store.get("job_t1"))!;
    assert.equal(j.status, "aborted");
    assert.equal(platinum.last.deleted, true);
  });

  test("typescript bots need a configured pipe command", async () => {
    const { store, run } = await setup({ onProcess: goodWorker() }, { language: "typescript", entry: "bot.ts" });
    await run();
    assert.match((await store.get("job_t1"))!.error!, /no pipe command for typescript/);
  });
});

describe("provisioning failures", () => {
  test("sandbox create failing is infra and leaves nothing to delete", async () => {
    const { platinum, store, run } = await setup({ createError: new Error("no capacity") });
    await run();
    const j = (await store.get("job_t1"))!;
    assert.deepEqual([j.status, j.errorClass], ["failed", "infra"]);
    assert.match(j.error!, /no capacity/);
    assert.equal(platinum.sandboxes.length, 0);
  });

  test("a corrupt artifact or a missing entry file is the user's (bot) error", async () => {
    let n = 0;
    const { platinum, store, run } = await setup({ sh: (s) => (s.includes("tar xzf") ? { exitCode: 2, stdout: "", stderr: "gzip: not in gzip format" } : (n++, undefined)) });
    await run();
    const j = (await store.get("job_t1"))!;
    assert.deepEqual([j.status, j.errorClass], ["failed", "bot"]);
    assert.match(j.error!, /could not unpack/);
    assert.equal(platinum.last.deleted, true);

    const b = await setup({ sh: (s) => (s.startsWith("test -f") ? { exitCode: 1, stdout: "", stderr: "" } : undefined) });
    await b.run();
    assert.match((await b.store.get("job_t1"))!.error!, /entry file bot.py not found/);
  });

  test("pip: PyPI is allowed only while installing, then egress goes back to the base list", async () => {
    const { platinum, store, run } = await setup({ onProcess: goodWorker() }, { requirementsFile: "requirements.txt" });
    await run();
    assert.equal((await store.get("job_t1"))!.status, "finished");
    const sbx = platinum.last;
    assert.deepEqual(sbx.spec.allowOut, ["pypi.org", "files.pythonhosted.org"]);
    assert.deepEqual(sbx.egress, [[]]);
    const pip = sbx.shCalls.find((s) => s.includes("pip install"))!;
    assert.match(pip, /cp -r \/opt\/sharingan\/bot \/tmp\/pip-src .* && cd \/tmp\/pip-src && setpriv --reuid=10002 --regid=10002 --clear-groups --inh-caps=-all --bounding-set=-all env -i .* pip install .* --target \/opt\/sharingan\/deps -r '\/tmp\/pip-src\/requirements\.txt'/, "the install runs unprivileged, no capabilities, in a private copy");
    const seal = sbx.shCalls.findIndex((s) => s.includes("sharingan-scrub 10002"));
    assert.ok(seal > sbx.shCalls.indexOf(pip), "after the install: deps sealed read-only and the install user wiped");
    assert.match(sbx.shCalls[seal]!, /^sharingan-scrub 10002 --keep \/opt\/sharingan\/deps && chown -R root:root \/opt\/sharingan\/deps && chmod -R a\+rX,go-w/, "wipe the install user first, then seal");
  });

  test("a failing pip install is the bot's error and the sandbox is still deleted", async () => {
    const { platinum, store, run } = await setup({ sh: (s) => (s.includes("pip install") ? { exitCode: 1, stdout: "", stderr: "No matching distribution for foo" } : undefined) }, { requirementsFile: "requirements.txt" });
    await run();
    const j = (await store.get("job_t1"))!;
    assert.deepEqual([j.status, j.errorClass], ["failed", "bot"]);
    assert.match(j.error!, /pip install failed.*foo/s);
    assert.equal(platinum.last.deleted, true);
  });

  test("a delete that fails is recorded (the reaper will retry) and does not mask the result", async () => {
    const { store, run } = await setup({ onProcess: goodWorker(), deleteError: new Error("api down") });
    await run();
    const j = (await store.get("job_t1"))!;
    assert.equal(j.status, "finished");
    assert.equal(j.sandboxDeleted, false);
  });

  test("oversized artifacts are refused", async () => {
    const { store, run } = await setup({ onProcess: goodWorker() }, {}, { MAX_ARTIFACT_BYTES: "2" });
    await run();
    assert.match((await store.get("job_t1"))!.error!, /artifact too large/);
  });
});

describe("connect mode (remote-bot path)", () => {
  const connectBehavior = (finishOnExit = false): Behavior => ({
    onProcess: (p) => { arena.connected = true; if (finishOnExit) setTimeout(() => p.exit(1, "bot crashed\n"), 40); },
  });

  test("happy path: bot identity, egress to the arena only, hosted run with the RUNNER token, polled to completion", async () => {
    arena.pollsToFinish = 3;
    const { platinum, store, run } = await setup(connectBehavior(), { mode: "connect", hands: 2000, lanes: 4 });
    await run();
    const j = (await store.get("job_t1"))!;
    assert.equal(j.status, "finished");
    assert.equal(j.verified, true);
    assert.equal(j.handsDone, 2004);
    assert.equal(j.mbbPerGame, 42.5);
    assert.equal(j.arenaPlayerId, "p_test");
    const start = arena.called("POST", "/v1/benchmarks/ladder-v1/runs")[0]!;
    assert.equal(start.auth, "Bearer runner-secret");
    assert.deepEqual(start.body, { player_id: "p_test", hands: 2000, lanes: 4, hosted: true });
    const sbx = platinum.last;
    assert.deepEqual(sbx.spec.allowOut, [new URL(arena.url).hostname]);
    const env = sbx.procs[0]!.opts.env!;
    assert.equal(env.ARENA_TOKEN, "sk_bot_token");
    assert.equal(env.BOT_FILE, "/opt/sharingan/bot/bot.py");
    assert.equal(sbx.procs[0]!.opts.cmd.slice(0, 4).join(" "), "setpriv --reuid=10002 --regid=10002 --clear-groups");
    assert.equal(sbx.deleted, true);
  });

  test("a bot that never connects fails as bot after the connect timeout", async () => {
    const { platinum, store, run } = await setup({ onProcess: (p) => { p.stderr = "ImportError: no module foo\n"; } }, { mode: "connect" }, { CONNECT_TIMEOUT_S: "0.1" });
    await run();
    const j = (await store.get("job_t1"))!;
    assert.deepEqual([j.status, j.errorClass], ["failed", "bot"]);
    assert.match(j.error!, /did not connect.*ImportError/s);
    assert.equal(platinum.last.deleted, true);
  });

  test("a bot process dying mid-run aborts the run and fails as bot, billing for hands played so far", async () => {
    arena.pollsToFinish = 1000;
    const { platinum, store, run } = await setup(connectBehavior(true), { mode: "connect", hands: 600 });
    await run();
    const j = (await store.get("job_t1"))!;
    assert.deepEqual([j.status, j.errorClass], ["failed", "bot"]);
    assert.match(j.error!, /exited mid-run.*bot crashed/s);
    assert.ok(arena.called("POST", "/v1/benchmarks/ladder-v1/runs/r_test/abort").length >= 1);
    assert.equal(platinum.last.deleted, true);
  });

  test("the arena refusing the run is infra (4xx)", async () => {
    arena.connected = true;
    arena.startStatus = 400;
    const { store, run } = await setup({ onProcess: () => {} }, { mode: "connect" });
    await run();
    const j = (await store.get("job_t1"))!;
    assert.deepEqual([j.status, j.errorClass], ["failed", "infra"]);
    assert.match(j.error!, /arena refused the run/);
  });

  test("abort stops the arena run too", async () => {
    arena.pollsToFinish = 1000;
    const { store, run, ac, platinum } = await setup(connectBehavior(), { mode: "connect" });
    const p = run();
    await until(async () => (await store.get("job_t1"))?.status === "running");
    ac.abort();
    await p;
    assert.equal((await store.get("job_t1"))!.status, "aborted");
    assert.equal(platinum.last.deleted, true);
  });
});

describe("helpers", () => {
  test("shq quotes anything", () => {
    assert.equal(shq("a'b $c"), `'a'\\''b $c'`);
  });
  test("renderCommand quotes values and rejects unknown placeholders", () => {
    assert.equal(renderCommand("x --a {a} --b {b}", { a: "one two", b: 3 }), "x --a 'one two' --b '3'");
    assert.throws(() => renderCommand("x {nope}", {}), /\{nope\}/);
  });
  test("botCommand: clean env, entry quoted, no uid of its own (the worker sets one per rotation)", () => {
    const c = botCommand(cfg, input({ entry: "my bot.py" }));
    assert.match(c, /^env -i HOME=\/tmp .* sharingan pipe '\/opt\/sharingan\/bot\/my bot\.py'$/);
    assert.doesNotMatch(c, /setpriv/);
  });
  test("parseProgress ignores long lines (a bot's stderr can't stall the runner with a crafted line)", () => {
    const t0 = Date.now();
    assert.equal(parseProgress("1".repeat(65_000) + "\n"), undefined);
    assert.ok(Date.now() - t0 < 50, `took ${Date.now() - t0} ms`);
    assert.deepEqual(parseProgress("x\n  sharingan-1 2007/20004 hands (2737 hands/s)\n"), { done: 2007, total: 20004 });
  });
  test("parseProgress: JSON lines and 'N/M hands'", () => {
    assert.deepEqual(parseProgress('noise\n{"hands_done":120,"hands_total":600}\n'), { done: 120, total: 600 });
    assert.deepEqual(parseProgress('{"type":"progress","data":{"hands_done":6,"hands_total":12}}'), { done: 6, total: 12 });
    assert.deepEqual(parseProgress("  progress 150/600 hands (25%)"), { done: 150, total: 600 });
    assert.equal(parseProgress("nothing here"), undefined);
  });
  test("reapOrphans deletes unowned run sandboxes older than the grace period, keeps owned and fresh ones", async () => {
    const platinum = new FakePlatinum({});
    const store = new MemoryJobStore();
    const mk = async (jobId: string) => (await platinum.createSandbox({ name: jobId, template: "t", cpu: 1, ramMb: 1, diskGb: 1, autoStopMinutes: 1, metadata: { sharingan_job: jobId }, allowOut: [] })).id;
    const orphan = await mk("job_orphan");
    const owned = await mk("job_owned");
    await store.put({ id: "job_owned", input: input({ idempotencyKey: "own" }), status: "running", createdAt: 1, reservedMicro: 1, settled: false, sandboxId: owned });
    const n = await reapOrphans({ platinum, store, log: noopLog }, 0);
    assert.equal(n, 1);
    assert.equal(platinum.sandboxes.find((s) => s.id === orphan)!.deleted, true);
    assert.equal(platinum.sandboxes.find((s) => s.id === owned)!.deleted, false);
  });
});
