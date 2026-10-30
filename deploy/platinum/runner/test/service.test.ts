// Service + HTTP app over fakes: the credit lifecycle around a job (reserve -> run -> settle), Stripe webhook top-ups,
// refunds when we fail, idempotency, recovery after a crash.
import assert from "node:assert/strict";
import type { AddressInfo } from "node:net";
import { afterEach, beforeEach, describe, test } from "node:test";
import { createApp } from "../src/app.ts";
import { ArenaClient } from "../src/arena.ts";
import type { Config } from "../src/config.ts";
import { MemoryLedger } from "../src/ledger.ts";
import { noopLog } from "../src/log.ts";
import { Service } from "../src/service.ts";
import { MemoryJobStore } from "../src/store.ts";
import { CREDITS_KIND, signStripePayload, SimulatedCheckoutGateway } from "../src/stripe.ts";
import { AUTH, FakeArena, FakePlatinum, tgzB64, until, WEBHOOK_SECRET, workerResult, testConfig, type Behavior } from "./helpers.ts";

let arena: FakeArena;
let cfg: Config;
let ledger: MemoryLedger;
let store: MemoryJobStore;
let platinum: FakePlatinum;
let svc: Service;
let server: ReturnType<typeof createApp>;
let base: string;

const goodWorker: Behavior["onProcess"] = (p, sbx) => setTimeout(() => { sbx.files.set("/work/result.json", new TextEncoder().encode(workerResult({ hands: 600 }))); p.exit(0); }, 20);

async function teardown() {
  svc?.stop();
  if (server?.listening) await new Promise((r) => server.close(r));
  server?.closeAllConnections?.();
  if (arena) await arena.stop().catch(() => {});
}

async function boot(behavior: Behavior = { onProcess: goodWorker }, env: Record<string, string> = {}) {
  await teardown();
  arena = new FakeArena();
  await arena.start();
  cfg = testConfig(arena.url, env);
  ledger = new MemoryLedger();
  store = new MemoryJobStore();
  platinum = new FakePlatinum(behavior);
  svc = new Service({ cfg, ledger, store, platinum, arena: new ArenaClient(cfg.arena), gateway: new SimulatedCheckoutGateway("http://runner.test"), log: noopLog });
  server = createApp(svc, cfg, { ready: async () => {}, log: noopLog });
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
  base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
}
afterEach(teardown);

const api = async (method: string, path: string, body?: unknown, headers: Record<string, string> = { authorization: `Bearer ${AUTH}` }) => {
  const res = await fetch(base + path, { method, headers: { "content-type": "application/json", ...headers }, body: body === undefined ? undefined : typeof body === "string" ? body : JSON.stringify(body) });
  const text = await res.text();
  return { status: res.status, json: text ? JSON.parse(text) : undefined };
};

const submitBody = (over: Record<string, unknown> = {}) => ({
  userId: "user_a", botName: "mybot", language: "python", entry: "bot.py", track: "ladder-v1", hands: 600, idempotencyKey: "idem-1",
  artifactTgzBase64: tgzB64(), ...over,
});

function stripeEvent(over: { id?: string; session?: string; cents?: number; user?: string; meta?: Record<string, string> } = {}) {
  const raw = JSON.stringify({
    id: over.id ?? "evt_1", type: "checkout.session.completed",
    data: { object: { id: over.session ?? "cs_test_1", payment_status: "paid", currency: "usd", amount_total: over.cents ?? 1000, client_reference_id: over.user ?? "user_a", metadata: { kind: CREDITS_KIND, user_id: over.user ?? "user_a", ...over.meta } } },
  });
  return { raw, sig: signStripePayload(raw, WEBHOOK_SECRET, Math.floor(Date.now() / 1000)) };
}
const topup = async (cents = 1000, over: Parameters<typeof stripeEvent>[0] = {}) => {
  const e = stripeEvent({ cents, ...over });
  return api("POST", "/v1/webhooks/stripe", e.raw, { "stripe-signature": e.sig, "content-type": "application/json" });
};
const waitSettled = async (id: string) => { await until(async () => (await store.get(id))?.settled === true, 4000, `job ${id} settled`); return (await store.get(id))!; };

describe("Stripe webhook", () => {
  beforeEach(() => boot());

  test("signed checkout.session.completed credits the user; replay is a no-op", async () => {
    const r1 = await topup(1000);
    assert.equal(r1.status, 200);
    assert.equal(r1.json.outcome, "credited");
    assert.equal((await api("GET", "/v1/credits/user_a")).json.balance_usd, 10);
    const r2 = await topup(1000);
    assert.equal(r2.json.outcome, "duplicate");
    assert.equal((await api("GET", "/v1/credits/user_a")).json.balance_usd, 10);
    const entries = (await api("GET", "/v1/credits/user_a/entries")).json.entries;
    assert.deepEqual(entries.map((e: any) => [e.kind, e.amount_usd]), [["topup", 10]]);
  });

  test("bad, missing or stale signatures are rejected and credit nothing", async () => {
    const e = stripeEvent();
    for (const sig of [undefined, "t=1,v1=abc", signStripePayload(e.raw, "whsec_wrong", Math.floor(Date.now() / 1000)), signStripePayload(e.raw, WEBHOOK_SECRET, 1_000_000)]) {
      const r = await api("POST", "/v1/webhooks/stripe", e.raw, { ...(sig ? { "stripe-signature": sig } : {}), "content-type": "application/json" });
      assert.equal(r.status, 400);
      assert.equal(r.json.error.code, "bad_signature");
    }
    const tampered = await api("POST", "/v1/webhooks/stripe", e.raw.replace("1000", "9999999"), { "stripe-signature": e.sig });
    assert.equal(tampered.status, 400);
    assert.equal((await api("GET", "/v1/credits/user_a")).json.balance_micro, 0);
  });

  test("the webhook needs no bearer token, everything else does", async () => {
    assert.equal((await topup(500)).status, 200);
    assert.equal((await api("GET", "/v1/credits/user_a", undefined, {})).status, 401);
    assert.equal((await api("GET", "/v1/credits/user_a", undefined, { authorization: "Bearer nope" })).status, 401);
    assert.equal((await api("POST", "/v1/jobs", submitBody(), {})).status, 401);
  });

  test("a validly signed event for another product is acknowledged (200) and ignored", async () => {
    const raw = JSON.stringify({ id: "evt_x", type: "checkout.session.completed", data: { object: { id: "cs_other", payment_status: "paid", currency: "usd", amount_total: 100, metadata: {} } } });
    const r = await api("POST", "/v1/webhooks/stripe", raw, { "stripe-signature": signStripePayload(raw, WEBHOOK_SECRET, Math.floor(Date.now() / 1000)) });
    assert.deepEqual([r.status, r.json.outcome], [200, "ignored"]);
  });

  test("checkout endpoint (simulated gateway) returns a session and validates input", async () => {
    const ok = await api("POST", "/v1/credits/checkout", { userId: "user_a", pack: "plus" });
    assert.equal(ok.status, 201);
    assert.match(ok.json.id, /^cs_sim_/);
    assert.deepEqual([ok.json.simulated, ok.json.price_cents, ok.json.credit_micro], [true, 5000, 55_000_000]);
    assert.equal((await api("POST", "/v1/credits/checkout", { userId: "user_a", pack: "nope" })).status, 400);
    assert.equal((await api("POST", "/v1/credits/checkout", { userId: "user_a", amountUsd: 1 })).status, 400);
    assert.equal((await api("POST", "/v1/credits/checkout", { amountUsd: 20 })).status, 400);
  });
});

describe("job lifecycle with credits", () => {
  beforeEach(() => boot());

  test("quote -> top up -> submit -> finished: debited by hands played, rest released, sandbox gone", async () => {
    assert.equal((await api("GET", "/v1/quote?hands=600")).json.price_usd, 0.6);
    await topup(1000);                                           // $10
    const sub = await api("POST", "/v1/jobs", submitBody());
    assert.equal(sub.status, 202);
    assert.equal(sub.json.reserved_micro, 600_000);              // 600 hands at $1 per 1,000
    const j = await waitSettled(sub.json.id);
    const view = (await api("GET", `/v1/jobs/${sub.json.id}`)).json;
    assert.equal(view.status, "finished");
    assert.equal(view.verified, true);
    assert.equal(view.hands_done, 600);
    assert.equal(view.charge_micro, 600_000);
    assert.equal(view.released_micro, 0);
    assert.equal(view.sandbox_deleted, true);
    assert.equal(j.settled, true);
    const c = (await api("GET", "/v1/credits/user_a")).json;
    assert.deepEqual([c.balance_usd, c.reserved_usd], [9.4, 0]);
    assert.equal(platinum.live.length, 0);
    assert.deepEqual((await api("GET", "/v1/credits/user_a/entries")).json.entries.map((e: any) => e.kind).reverse(), ["topup", "reserve", "settle"]);
  });

  test("charge follows hands actually played; the unused part of the hold is released", async () => {
    await topup(1000);
    arena.handsPlayed = 300;                                     // the arena says only 300 hands were played
    const sub = await api("POST", "/v1/jobs", submitBody({ idempotencyKey: "part" }));
    const j = await waitSettled(sub.json.id);
    assert.deepEqual([j.status, j.handsDone, j.chargeMicro, j.releasedMicro], ["finished", 300, 300_000, 300_000]);
    assert.equal((await api("GET", "/v1/credits/user_a")).json.balance_usd, 9.7);
  });

  test("not enough credits: 402, nothing reserved, no sandbox created", async () => {
    await topup(50);                                             // $0.50 < $0.60
    const r = await api("POST", "/v1/jobs", submitBody());
    assert.equal(r.status, 402);
    assert.equal(r.json.error.code, "insufficient_credits");
    assert.deepEqual([platinum.sandboxes.length, (await api("GET", "/v1/credits/user_a")).json.reserved_micro], [0, 0]);
    assert.equal(await store.byKey("idem-1"), undefined);
  });

  test("submit is idempotent: same key -> same job, one reservation, one run", async () => {
    await topup(1000);
    const a = await api("POST", "/v1/jobs", submitBody());
    const b = await api("POST", "/v1/jobs", submitBody());
    assert.equal(a.status, 202);
    assert.deepEqual([b.status, b.json.deduped, b.json.id], [200, true, a.json.id]);
    await waitSettled(a.json.id);
    assert.equal(platinum.sandboxes.length, 1);
    assert.equal(arena.called("POST", "/v1/results").length, 1);
    assert.equal((await api("GET", "/v1/credits/user_a")).json.balance_usd, 9.4);
    // the same key with a different size is a conflict, not a second reservation
    const c = await api("POST", "/v1/jobs", submitBody({ hands: 6000 }));
    assert.deepEqual([c.status, c.json.error.code], [409, "idempotency_conflict"]);   // never silently the old 600-hand job
  });

  test("OUR failure (sandbox create) refunds in full", async () => {
    await topup(1000);
    platinum.behavior.createError = new Error("no capacity");
    const sub = await api("POST", "/v1/jobs", submitBody());
    const j = await waitSettled(sub.json.id);
    assert.deepEqual([j.status, j.errorClass, j.chargeMicro, j.releasedMicro], ["failed", "infra", 0, 600_000]);
    assert.equal((await api("GET", "/v1/credits/user_a")).json.balance_usd, 10);
  });

  test("the arena rejecting the result (our signing/config) refunds in full", async () => {
    await topup(1000);
    arena.resultStatus = 403;
    const j = await waitSettled((await api("POST", "/v1/jobs", submitBody())).json.id);
    assert.deepEqual([j.status, j.errorClass, j.chargeMicro], ["failed", "infra", 0]);
  });

  test("a crashing bot with no hands played is free by default", async () => {
    platinum.behavior.onProcess = (p) => setTimeout(() => p.exit(1, "Traceback\n"), 10);
    await topup(1000);
    const j = await waitSettled((await api("POST", "/v1/jobs", submitBody())).json.id);
    assert.deepEqual([j.status, j.errorClass, j.chargeMicro], ["failed", "bot", 0]);
  });

  test("... and charged FAILED_BOT_CHARGE_USD when that is configured", async () => {
    await boot({ onProcess: (p) => setTimeout(() => p.exit(1, "Traceback\n"), 10) }, { FAILED_BOT_CHARGE_USD: "0.05" });
    await topup(1000);
    const k = await waitSettled((await api("POST", "/v1/jobs", submitBody({ idempotencyKey: "kk" }))).json.id);
    assert.deepEqual([k.status, k.chargeMicro, k.releasedMicro], ["failed", 50_000, 550_000]);
  });

  test("abort: the job ends aborted, hands played are charged, the rest released", async () => {
    platinum.behavior.onProcess = () => {};
    await topup(1000);
    const sub = await api("POST", "/v1/jobs", submitBody());
    await until(async () => (await store.get(sub.json.id))?.status === "running");
    assert.equal((await api("POST", `/v1/jobs/${sub.json.id}/abort`)).status, 202);
    const j = await waitSettled(sub.json.id);
    assert.deepEqual([j.status, j.chargeMicro, j.releasedMicro], ["aborted", 0, 600_000]);
    assert.equal(platinum.live.length, 0);
  });

  test("concurrency cap: extra jobs wait for a slot and still complete", async () => {
    await boot({ onProcess: goodWorker }, { MAX_CONCURRENT_RUNS: "1" });
    await topup(1000);
    const ids = [];
    for (let i = 0; i < 3; i++) ids.push((await api("POST", "/v1/jobs", submitBody({ idempotencyKey: `c${i}` }))).json.id);
    let maxLive = 0;
    const t = setInterval(() => { maxLive = Math.max(maxLive, platinum.live.length); }, 2);
    for (const id of ids) await waitSettled(id);
    clearInterval(t);
    assert.ok(maxLive <= 1, `saw ${maxLive} live sandboxes`);
    assert.equal((await api("GET", "/v1/credits/user_a")).json.balance_usd, 8.2);
  });

  test("validation: bad input is a 400 and never reserves credits", async () => {
    await topup(1000);
    for (const bad of [{ track: "nope" }, { hands: 5 }, { hands: 10_000_000 }, { entry: "../x.py" }, { entry: "/abs.py" }, { language: "ruby" }, { userId: "a b" }, { lanes: 99 }, { artifactTgzBase64: "" }, { mode: "magic" }, { language: "typescript", entry: "bot.ts" }]) {
      const r = await api("POST", "/v1/jobs", submitBody({ idempotencyKey: `v${JSON.stringify(bad)}`, ...bad }));
      assert.equal(r.status, 400, JSON.stringify(bad));
    }
    assert.equal((await api("GET", "/v1/credits/user_a")).json.reserved_micro, 0);
    assert.equal(platinum.sandboxes.length, 0);
    assert.equal((await api("POST", "/v1/jobs", "{nope")).status, 400);
    assert.equal((await api("GET", "/v1/jobs/job_missing")).status, 404);
  });

  test("validation: an artifact that is not a gzip tar with the entry file is a 400 before any credits are reserved", async () => {
    await topup(1000);
    for (const over of [
      { artifactTgzBase64: Buffer.from("fake-tgz").toString("base64") },
      { artifactTgzBase64: "!!!" },
      { artifactTgzBase64: tgzB64({ "other.py": "x" }) },
    ]) assert.equal((await api("POST", "/v1/jobs", submitBody({ idempotencyKey: JSON.stringify(over).slice(0, 40), ...over }))).status, 400);
    assert.equal((await api("GET", "/v1/credits/user_a")).json.reserved_micro, 0);
  });

  test("hands: 601 is echoed as 606 (submit answer, job, hands_total), reserved and billed for 606", async () => {
    await topup(1000);
    const sub = await api("POST", "/v1/jobs", submitBody({ hands: 601 }));
    assert.equal(sub.json.hands, 606);
    assert.equal(sub.json.reserved_micro, 606_000);
    const j = (await api("GET", `/v1/jobs/${sub.json.id}`)).json;
    assert.deepEqual([j.hands, j.hands_total], [606, 606]);
  });

  test("CORS headers are on every response and OPTIONS needs no auth (service app)", async () => {
    const pre = await fetch(base + "/v1/jobs", { method: "OPTIONS" });
    assert.equal(pre.status, 204);
    assert.equal(pre.headers.get("access-control-allow-origin"), "*");
    const unauth = await fetch(base + "/v1/jobs/x");
    assert.equal(unauth.status, 401);
    assert.equal(unauth.headers.get("access-control-allow-origin"), "*");
    assert.equal((await fetch(base + "/healthz")).headers.get("access-control-allow-origin"), "*");
  });

  test("healthz / readyz are open; unknown routes 404", async () => {
    assert.equal((await api("GET", "/healthz", undefined, {})).status, 200);
    assert.equal((await api("GET", "/readyz", undefined, {})).status, 200);
    assert.equal((await api("GET", "/nope")).status, 404);
  });

  test("listing jobs per user", async () => {
    await topup(2000);
    for (const k of ["l1", "l2"]) await waitSettled((await api("POST", "/v1/jobs", submitBody({ idempotencyKey: k }))).json.id);
    assert.equal((await api("GET", "/v1/jobs?userId=user_a")).json.jobs.length, 2);
    assert.equal((await api("GET", "/v1/jobs?userId=user_b")).json.jobs.length, 0);
    assert.equal((await api("GET", "/v1/jobs")).status, 400);
  });
});

describe("crash recovery", () => {
  beforeEach(() => boot());

  test("a job that was running when the runner died is failed, its sandbox deleted and the hold refunded", async () => {
    await ledger.topup("user_a", 10_000_000, "seed");
    await ledger.reserve("user_a", "job:crashed", 600_000);
    const sbx = await platinum.createSandbox({ name: "x", template: "t", cpu: 1, ramMb: 1, diskGb: 1, autoStopMinutes: 1, metadata: { sharingan_job: "job_c" }, allowOut: [] });
    await store.put({
      id: "job_c", status: "running", createdAt: Date.now(), reservedMicro: 600_000, settled: false, sandboxId: sbx.id,
      input: { userId: "user_a", botName: "b", language: "python", entry: "bot.py", track: "ladder-v1", hands: 600, idempotencyKey: "crashed", mode: "worker" },
    });
    const r = await svc.recover();
    assert.equal(r.failed, 1);
    const j = (await store.get("job_c"))!;
    assert.deepEqual([j.status, j.errorClass, j.settled, j.chargeMicro, j.releasedMicro], ["failed", "infra", true, 0, 600_000]);
    assert.equal((platinum.sandboxes[0] as any).deleted, true);
    assert.deepEqual(await ledger.balance("user_a"), { userId: "user_a", balanceMicro: 10_000_000, reservedMicro: 0 });
  });

  test("a finished-but-unsettled job (crash between teardown and settle) is settled on boot, once", async () => {
    await ledger.topup("user_a", 10_000_000, "seed");
    await ledger.reserve("user_a", "job:done", 600_000);
    await store.put({
      id: "job_d", status: "finished", createdAt: Date.now(), reservedMicro: 600_000, settled: false, handsDone: 600, verified: true,
      input: { userId: "user_a", botName: "b", language: "python", entry: "bot.py", track: "ladder-v1", hands: 600, idempotencyKey: "done", mode: "worker" },
    });
    await svc.recover();
    await svc.recover();
    assert.deepEqual(await ledger.balance("user_a"), { userId: "user_a", balanceMicro: 9_400_000, reservedMicro: 0 });
    assert.equal((await store.get("job_d"))!.chargeMicro, 600_000);
  });

  test("a stale reservation with no job is released", async () => {
    let now = Date.now();
    const old = new MemoryLedger(() => now);
    svc = new Service({ ...svc.d, ledger: old });
    await old.topup("user_a", 5_000_000, "seed");
    await old.reserve("user_a", "job:ghost", 1_000_000);
    now += 24 * 3600 * 1000;
    const r = await svc.recover();
    assert.equal(r.released, 1);
    assert.deepEqual(await old.balance("user_a"), { userId: "user_a", balanceMicro: 5_000_000, reservedMicro: 0 });
  });
});
