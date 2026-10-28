// BILLING_MODE=free: no ledger, per-player quotas, public players authenticate with their ARENA bot token.
import assert from "node:assert/strict";
import type { AddressInfo } from "node:net";
import { afterEach, describe, test } from "node:test";
import { createApp } from "../src/app.ts";
import { ArenaClient } from "../src/arena.ts";
import { MemoryLedger } from "../src/ledger.ts";
import { noopLog } from "../src/log.ts";
import { Service } from "../src/service.ts";
import { MemoryJobStore } from "../src/store.ts";
import { SimulatedCheckoutGateway } from "../src/stripe.ts";
import { AUTH, FakeArena, FakePlatinum, until, workerResult, testConfig, type Behavior } from "./helpers.ts";

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
const sub = (over: Record<string, unknown> = {}) => ({ botName: "b", language: "python", entry: "bot.py", track: "ladder-v1", hands: 600, idempotencyKey: "k1", artifactTgzBase64: Buffer.from("x").toString("base64"), ...over });

describe("free billing mode", () => {
  test("arena bot token authenticates; job is free, scoped to the player, ledger untouched", async () => {
    await boot({ onProcess: good });
    assert.equal((await api("POST", "/v1/jobs", "sk_bad", sub())).status, 401);
    const r = await api("POST", "/v1/jobs", "sk_user_alice", sub({ userId: "someone_else" }));
    assert.equal(r.status, 202);
    const id = r.json.id;
    await until(async () => (await api("GET", `/v1/jobs/${id}`, "sk_user_alice")).json.settled === true);
    const j = (await api("GET", `/v1/jobs/${id}`, "sk_user_alice")).json;
    assert.equal(j.status, "finished"); assert.equal(j.user_id, "pl_alice"); assert.equal(j.charge_micro, 0); assert.equal(j.verified, true);
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
      await until(async () => (await api("GET", `/v1/jobs/${r.json.id}`, "sk_user_carol")).json.settled === true);
    }
    const over = await api("POST", "/v1/jobs", "sk_user_carol", sub({ idempotencyKey: "c" }));
    assert.equal(over.status, 429); assert.equal(over.json.error.code, "quota_hands_per_day");
  });

  test("webhook route is off when billing is free and no secret is set", async () => {
    await boot({ onProcess: good });
    assert.equal((await api("POST", "/v1/webhooks/stripe", undefined, {})).status, 401);
  });
});
