// BILLING_MODE=free: no ledger, per-player quotas, public players authenticate with their ARENA bot token.
import assert from "node:assert/strict";
import type { AddressInfo } from "node:net";
import { afterEach, describe, test } from "node:test";
import { createApp, jobView, publicJobView } from "../src/app.ts";
import { ArenaClient } from "../src/arena.ts";
import { MemoryLedger } from "../src/ledger.ts";
import { noopLog } from "../src/log.ts";
import { Service } from "../src/service.ts";
import { MemoryJobStore, type Job } from "../src/store.ts";
import { SimulatedCheckoutGateway } from "../src/stripe.ts";
import { AUTH, FakeArena, FakePlatinum, tgzB64, until, workerResult, testConfig, type Behavior } from "./helpers.ts";

let arena: FakeArena; let svc: Service; let server: ReturnType<typeof createApp>; let base: string; let ledger: MemoryLedger; let store: MemoryJobStore;
const good: Behavior["onProcess"] = (p, sbx) => setTimeout(() => { sbx.files.set("/work/result.json", new TextEncoder().encode(workerResult({ hands: 600 }))); p.exit(0); }, 30);
const hang: Behavior["onProcess"] = () => {};

async function boot(behavior: Behavior, env: Record<string, string> = {}) {
  arena = new FakeArena(); await arena.start();
  const cfg = testConfig(arena.url, { BILLING_MODE: "free", STRIPE_WEBHOOK_SECRET: "", FREE_MAX_HANDS_PER_DAY: "1200", FREE_MAX_CONCURRENT_JOBS: "2", ...env });
  ledger = new MemoryLedger(); store = new MemoryJobStore();
  const ac = new ArenaClient(cfg.arena);
  svc = new Service({ cfg, ledger, store, platinum: new FakePlatinum(behavior), arena: ac, gateway: new SimulatedCheckoutGateway("http://x"), log: noopLog });
  server = createApp(svc, cfg, { ready: async () => {}, log: noopLog, arena: ac });
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
  base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
}
afterEach(async () => { for (const j of await store.active()) await svc.abort(j.id).catch(() => {}); await svc?.drain(); svc?.stop(); server?.closeAllConnections?.(); await new Promise((r) => server?.close(r)); await arena?.stop().catch(() => {}); });

const api = async (method: string, path: string, token: string | undefined, body?: unknown) => {
  const res = await fetch(base + path, { method, headers: { "content-type": "application/json", ...(token ? { authorization: `Bearer ${token}` } : {}) }, body: body === undefined ? undefined : JSON.stringify(body) });
  const t = await res.text();
  return { status: res.status, json: t ? JSON.parse(t) : undefined };
};
const sub = (over: Record<string, unknown> = {}) => ({ botName: "b", language: "python", entry: "bot.py", track: "ladder-v1", hands: 600, idempotencyKey: "k1", artifactTgzBase64: tgzB64(), ...over });

describe("free billing mode", () => {
  test("arena bot token authenticates; job is free, scoped to the player, ledger untouched", async () => {
    await boot({ onProcess: good });
    assert.equal((await api("POST", "/v1/jobs", "sk_bad", sub())).status, 401);
    const r = await api("POST", "/v1/jobs", "sk_user_alice", sub({ userId: "someone_else" }));
    assert.equal(r.status, 202);
    const id = r.json.id;
    await until(async () => (await store.get(id))?.settled === true);
    const j = (await api("GET", `/v1/jobs/${id}`, AUTH)).json;     // service view (the player's own view is the trimmed public one)
    assert.equal(j.status, "finished"); assert.equal(j.user_id, "pl_alice"); assert.equal(j.charge_micro, 0); assert.equal(j.verified, true);
    assert.equal(j.arena_player_id, "pl_alice", "credited to the submitting player (one leaderboard row per bot)");
    assert.ok(!arena.calls.some((c) => c.path === "/v1/bots"), "no fresh identity registered");
    assert.equal((await api("GET", `/v1/jobs/${id}`, "sk_user_bob")).status, 404, "other players cannot see it");
    assert.equal((await api("GET", `/v1/jobs/${id}`, AUTH)).status, 200, "service token can");
    assert.deepEqual((await api("GET", "/v1/jobs", "sk_user_alice")).json.jobs.map((x: any) => x.id), [id]);
    assert.equal((await api("GET", "/v1/jobs", "sk_user_bob")).json.jobs.length, 0);
    assert.equal((await ledger.balance("pl_alice")).balanceMicro, 0);
    assert.equal((await api("GET", "/v1/credits/pl_bob", "sk_user_alice")).status, 403);
    assert.equal((await api("POST", "/v1/credits/checkout", "sk_user_alice", { userId: "pl_alice", amountUsd: 10 })).status, 403);
    assert.equal((await api("POST", "/v1/credits/checkout", AUTH, { userId: "pl_alice", amountUsd: 10 })).status, 400);
  });

  test("same idempotency key from two players is two jobs", async () => {
    await boot({ onProcess: good });
    const a = await api("POST", "/v1/jobs", "sk_user_alice", sub());
    const b = await api("POST", "/v1/jobs", "sk_user_bob", sub());
    assert.notEqual(a.json.id, b.json.id);
    assert.equal((await api("POST", "/v1/jobs", "sk_user_alice", sub())).json.deduped, true);
  });

  test("max 2 concurrent jobs per player, then 429; other players unaffected", async () => {
    await boot({ onProcess: hang });
    assert.equal((await api("POST", "/v1/jobs", "sk_user_alice", sub({ idempotencyKey: "a" }))).status, 202);
    assert.equal((await api("POST", "/v1/jobs", "sk_user_alice", sub({ idempotencyKey: "b" }))).status, 202);
    const third = await api("POST", "/v1/jobs", "sk_user_alice", sub({ idempotencyKey: "c" }));
    assert.equal(third.status, 429); assert.equal(third.json.error.code, "quota_concurrent");
    assert.equal((await api("POST", "/v1/jobs", "sk_user_bob", sub({ idempotencyKey: "a" }))).status, 202);
    const q = (await api("GET", "/v1/quota", "sk_user_alice")).json;
    assert.equal(q.concurrent_active, 2); assert.equal(q.hands_used_24h, 1200);
  });

  test("hands per day quota (rolling 24 h)", async () => {
    await boot({ onProcess: good }, { FREE_MAX_HANDS_PER_DAY: "1200" });
    for (const k of ["a", "b"]) {
      const r = await api("POST", "/v1/jobs", "sk_user_carol", sub({ idempotencyKey: k }));
      assert.equal(r.status, 202);
      await until(async () => (await store.get(r.json.id))?.settled === true);
    }
    const over = await api("POST", "/v1/jobs", "sk_user_carol", sub({ idempotencyKey: "c" }));
    assert.equal(over.status, 429); assert.equal(over.json.error.code, "quota_hands_per_day");
  });

  test("webhook route is off when billing is free and no secret is set", async () => {
    await boot({ onProcess: good });
    assert.equal((await api("POST", "/v1/webhooks/stripe", undefined, {})).status, 401);
  });

  test("CORS: every response has the headers, OPTIONS preflight is 204 without auth, on all public routes", async () => {
    await boot({ onProcess: good });
    for (const path of ["/v1/jobs", "/v1/jobs/job_x", "/v1/jobs/job_x/abort", "/v1/quota", "/nope"]) {
      const res = await fetch(base + path, { method: "OPTIONS", headers: { origin: "https://sharingan.poker", "access-control-request-method": "POST", "access-control-request-headers": "authorization,content-type" } });
      assert.equal(res.status, 204, path);
      assert.equal(await res.text(), "");
      assert.equal(res.headers.get("access-control-allow-origin"), "*");
      assert.equal(res.headers.get("access-control-allow-methods"), "GET, POST, DELETE, OPTIONS");
      assert.equal(res.headers.get("access-control-allow-headers"), "authorization, content-type");
      assert.equal(res.headers.get("access-control-max-age"), "86400");
    }
    const r202 = await fetch(base + "/v1/jobs", { method: "POST", headers: { authorization: "Bearer sk_user_alice", "content-type": "application/json" }, body: JSON.stringify(sub()) });
    const r401 = await fetch(base + "/v1/quota");
    const r404 = await fetch(base + "/v1/jobs/job_missing", { headers: { authorization: "Bearer sk_user_alice" } });
    const r200 = await fetch(base + "/v1/quota", { headers: { authorization: "Bearer sk_user_alice" } });
    for (const [res, code] of [[r202, 202], [r401, 401], [r404, 404], [r200, 200]] as const) {
      assert.equal(res.status, code);
      assert.equal(res.headers.get("access-control-allow-origin"), "*", `status ${code}`);
      assert.equal(res.headers.get("access-control-allow-headers"), "authorization, content-type");
    }
  });

  test("a public player sees only what a user needs; the service token still sees everything", async () => {
    await boot({ onProcess: good });
    const r = await api("POST", "/v1/jobs", "sk_user_alice", sub({ hands: 601 }));
    assert.equal(r.status, 202);
    assert.deepEqual(Object.keys(r.json).sort(), ["deduped", "hands", "id", "status"], "no reserved_micro in the submit answer");
    assert.equal(r.json.hands, 606, "the rounded value is echoed");
    const id = r.json.id;
    await until(async () => (await store.get(id))?.settled === true);
    const pub = (await api("GET", `/v1/jobs/${id}`, "sk_user_alice")).json;
    assert.deepEqual(Object.keys(pub).sort(), [
      "arena_player_id", "bot_log_tail", "bot_name", "ci95_mbb", "created_at", "deals_seed", "error", "finished_at", "forfeit_hands", "hands", "hands_done",
      "hands_total", "id", "mbb_per_game", "run_id", "status", "track", "verified",
    ]);
    assert.equal(pub.hands, 606);
    assert.equal(pub.status, "finished");
    assert.match(pub.deals_seed, /^run:[0-9a-f]{64}$/, "revealed after the job");
    assert.equal(pub.arena_player_id, "pl_alice");
    assert.equal(pub.run_id, "r_worker");
    const text = JSON.stringify(pub);
    for (const leak of ["sandbox", "cogs", "reserved", "charge", "platinum", "mode", "user_id", "settled", "released", "error_class"]) assert.ok(!text.includes(leak), `leaks ${leak}`);
    assert.deepEqual((await api("GET", "/v1/jobs", "sk_user_alice")).json.jobs, [pub], "the list uses the same public view");
    const full = (await api("GET", `/v1/jobs/${id}`, AUTH)).json;
    for (const k of ["sandbox_id", "cogs_micro", "reserved_micro", "charge_micro", "platinum_usd", "mode", "user_id"]) assert.ok(k in full, k);
    assert.equal(full.deals_seed, pub.deals_seed);
  });

  test("the deal seed is not shown before the job is over (either view); infra errors are generic for the public", () => {
    const base: Job = { id: "job_v", input: { userId: "pl_a", botName: "b", language: "python", entry: "bot.py", track: "ladder-v1", hands: 600, idempotencyKey: "k", mode: "worker" },
      status: "running", createdAt: Date.now(), reservedMicro: 0, settled: false, dealsSeed: "run:" + "ab".repeat(32), sandboxId: "sbx_1" };
    assert.equal(publicJobView(base).deals_seed, null);
    assert.equal(jobView(base).deals_seed, null);
    assert.equal(publicJobView({ ...base, status: "finished" }).deals_seed, base.dealsSeed);
    const infra = publicJobView({ ...base, status: "failed", errorClass: "infra", error: "arena accepted the result but did not mark it verified (check ARENA_WORKER_KEYS / WORKER_KEY_ID)" });
    assert.doesNotMatch(infra.error!, /ARENA_WORKER_KEYS|sandbox/);
    assert.equal(publicJobView({ ...base, status: "failed", errorClass: "bot", error: "the bot never answered a decision" }).error, "the bot never answered a decision");
  });

  test("cheap validation runs BEFORE the quota: garbage is a 400 even when the player is over quota or at the concurrency cap", async () => {
    await boot({ onProcess: hang });
    assert.equal((await api("POST", "/v1/jobs", "sk_user_dan", sub({ idempotencyKey: "a", hands: 1200 }))).status, 202);   // uses the whole daily quota
    assert.equal((await api("POST", "/v1/jobs", "sk_user_dan", sub({ idempotencyKey: "b", hands: 6 }))).status, 400);
    assert.equal((await api("POST", "/v1/jobs", "sk_user_dan", sub({ idempotencyKey: "c" }))).status, 429, "a valid request is over quota");
    const bad: Array<[string, Record<string, unknown>, RegExp]> = [
      ["not base64", { artifactTgzBase64: "@@@not base64@@@" }, /base64/],
      ["not gzip", { artifactTgzBase64: Buffer.from("plain text, not an archive").toString("base64") }, /gzip/],
      ["entry missing in the tar", { artifactTgzBase64: tgzB64({ "other.py": "x" }) }, /entry file bot\.py not found/],
      ["hands out of range", { hands: 10_000_000 }, /hands/],
      ["requirements file missing", { requirementsFile: "requirements.txt" }, /requirements file/],
    ];
    for (const [what, over, re] of bad) {
      const r = await api("POST", "/v1/jobs", "sk_user_dan", sub({ idempotencyKey: `bad-${what}`, ...over }));
      assert.equal(r.status, 400, what);
      assert.match(r.json.error.message, re, what);
    }
    assert.equal((await api("POST", "/v1/jobs", "sk_user_dan", sub({ idempotencyKey: "z", artifactTgzBase64: tgzB64({ "./bot.py": "ok", "requirements.txt": "" }), requirementsFile: "requirements.txt" }))).status, 429, "a good artifact reaches the quota check");
  });

  test("hands are echoed rounded up to a multiple of 6 and agree with hands_total", async () => {
    await boot({ onProcess: hang });
    const r = await api("POST", "/v1/jobs", "sk_user_eve", sub({ hands: 601 }));
    const j = (await api("GET", `/v1/jobs/${r.json.id}`, "sk_user_eve")).json;
    assert.deepEqual([r.json.hands, j.hands, j.hands_total], [606, 606, 606]);
    assert.equal((await api("GET", "/v1/quota", "sk_user_eve")).json.hands_used_24h, 606);
  });
  test("contact is private, the uploaded bot is kept, and only the service token can fetch it back", async () => {
    await boot({ onProcess: good });
    assert.equal((await api("POST", "/v1/jobs", "sk_user_alice", sub({ contact: 5 }))).status, 400);
    assert.equal((await api("POST", "/v1/jobs", "sk_user_alice", sub({ contact: "x".repeat(201) }))).status, 400);
    const art = tgzB64();
    const r = await api("POST", "/v1/jobs", "sk_user_alice", sub({ contact: "  @alice  ", artifactTgzBase64: art }));
    assert.equal(r.status, 202);
    const id = r.json.id;
    await until(async () => (await store.get(id))?.settled === true);
    assert.equal((await api("GET", `/v1/jobs/${id}`, AUTH)).json.contact, "@alice", "trimmed, visible to the organizers");
    assert.ok(!("contact" in (await api("GET", `/v1/jobs/${id}`, "sk_user_alice")).json), "not in the public view");
    assert.equal((await api("GET", `/v1/jobs/${id}/artifact`, "sk_user_alice")).status, 403);
    const res = await fetch(`${base}/v1/jobs/${id}/artifact`, { headers: { authorization: `Bearer ${AUTH}` } });
    assert.equal(res.status, 200);
    assert.equal(res.headers.get("content-type"), "application/gzip");
    assert.deepEqual(Buffer.from(await res.arrayBuffer()), Buffer.from(art, "base64"), "byte-identical to the upload");
    assert.equal((await api("GET", "/v1/jobs/job_nope/artifact", AUTH)).status, 404);
  });
});
