// One contract, two implementations: MemoryLedger always, PgLedger/PgJobStore when TEST_DATABASE_URL points at a
// throwaway Postgres (each run uses a fresh schema).
import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { after, before, describe, test } from "node:test";
import { createPool, migrate, type Pool } from "../src/db.ts";
import { InsufficientCredits, LedgerConflict, MemoryLedger, PgLedger, type Ledger } from "../src/ledger.ts";
import { MemoryJobStore, PgJobStore, type Job, type JobStore } from "../src/store.ts";

const DB = process.env.TEST_DATABASE_URL;

function ledgerContract(name: string, make: () => Promise<Ledger> | Ledger) {
  describe(`ledger contract: ${name}`, () => {
    test("top-up is idempotent on the event id", async () => {
      const l = await make();
      assert.equal((await l.topup("u", 5_000_000, "evt_1")).applied, true);
      assert.equal((await l.topup("u", 5_000_000, "evt_1")).applied, false);
      assert.deepEqual(await l.balance("u"), { userId: "u", balanceMicro: 5_000_000, reservedMicro: 0 });
      assert.deepEqual(await l.balance("nobody"), { userId: "nobody", balanceMicro: 0, reservedMicro: 0 });
    });

    test("reserve holds, settle charges and releases the rest", async () => {
      const l = await make();
      await l.topup("u", 10_000_000, "e");
      const r = await l.reserve("u", "job:k1", 4_000_000);
      assert.equal(r.created, true);
      assert.deepEqual(await l.balance("u"), { userId: "u", balanceMicro: 6_000_000, reservedMicro: 4_000_000 });
      const s = await l.settle("job:k1", 1_500_000, { hands: 1500 });
      assert.deepEqual([s.settledNow, s.chargedMicro, s.releasedMicro], [true, 1_500_000, 2_500_000]);
      assert.deepEqual(await l.balance("u"), { userId: "u", balanceMicro: 8_500_000, reservedMicro: 0 });
      const kinds = (await l.entries("u")).map((e) => `${e.kind}:${e.amountMicro}`);
      assert.deepEqual(kinds.reverse(), ["topup:10000000", "reserve:4000000", "settle:1500000", "release:2500000"]);
    });

    test("reserve is idempotent; a conflicting retry is refused", async () => {
      const l = await make();
      await l.topup("u", 10_000_000, "e");
      await l.reserve("u", "job:k", 3_000_000);
      const again = await l.reserve("u", "job:k", 3_000_000);
      assert.equal(again.created, false);
      assert.equal((await l.balance("u")).reservedMicro, 3_000_000);
      await assert.rejects(l.reserve("u", "job:k", 4_000_000), LedgerConflict);
      await assert.rejects(l.reserve("other", "job:k", 3_000_000), LedgerConflict);
    });

    test("settle is idempotent and never double-charges", async () => {
      const l = await make();
      await l.topup("u", 10_000_000, "e");
      await l.reserve("u", "job:k", 5_000_000);
      const a = await l.settle("job:k", 2_000_000);
      const b = await l.settle("job:k", 4_999_999);          // a replay with another number changes nothing
      assert.equal(a.settledNow, true);
      assert.equal(b.settledNow, false);
      assert.equal(b.chargedMicro, 2_000_000);
      assert.deepEqual(await l.balance("u"), { userId: "u", balanceMicro: 8_000_000, reservedMicro: 0 });
      await assert.rejects(l.settle("job:missing", 0), LedgerConflict);
    });

    test("charge is clamped to the reservation (no overdraft through settle)", async () => {
      const l = await make();
      await l.topup("u", 1_000_000, "e");
      await l.reserve("u", "job:k", 400_000);
      const s = await l.settle("job:k", 9_000_000);
      assert.equal(s.chargedMicro, 400_000);
      assert.deepEqual(await l.balance("u"), { userId: "u", balanceMicro: 600_000, reservedMicro: 0 });
    });

    test("an account can never go negative: reserve refuses with InsufficientCredits", async () => {
      const l = await make();
      await l.topup("u", 1_000_000, "e");
      await assert.rejects(l.reserve("u", "job:big", 1_000_001), (e: unknown) => e instanceof InsufficientCredits && e.balanceMicro === 1_000_000 && e.neededMicro === 1_000_001);
      await l.reserve("u", "job:a", 600_000);
      await assert.rejects(l.reserve("u", "job:b", 600_000), InsufficientCredits);        // the first hold counts
      assert.deepEqual(await l.balance("u"), { userId: "u", balanceMicro: 400_000, reservedMicro: 600_000 });
    });

    test("concurrent reserves never oversell the balance", async () => {
      const l = await make();
      await l.topup("u", 5_000_000, "e");
      const results = await Promise.allSettled(Array.from({ length: 10 }, (_, i) => l.reserve("u", `job:c${i}`, 1_000_000)));
      assert.equal(results.filter((r) => r.status === "fulfilled").length, 5);
      assert.equal(results.filter((r) => r.status === "rejected").length, 5);
      assert.deepEqual(await l.balance("u"), { userId: "u", balanceMicro: 0, reservedMicro: 5_000_000 });
    });

    test("concurrent duplicate top-ups and settles apply once", async () => {
      const l = await make();
      const t = await Promise.all(Array.from({ length: 6 }, () => l.topup("u", 2_000_000, "evt_dup")));
      assert.equal(t.filter((x) => x.applied).length, 1);
      await l.reserve("u", "job:k", 2_000_000);
      const s = await Promise.all(Array.from({ length: 6 }, () => l.settle("job:k", 500_000)));
      assert.equal(s.filter((x) => x.settledNow).length, 1);
      assert.deepEqual(await l.balance("u"), { userId: "u", balanceMicro: 1_500_000, reservedMicro: 0 });
    });

    test("invariant: balance + reserved + charged == topped up", async () => {
      const l = await make();
      let topped = 0, charged = 0;
      for (let i = 0; i < 5; i++) { await l.topup("u", 3_000_000, `e${i}`); topped += 3_000_000; }
      for (let i = 0; i < 8; i++) {
        await l.reserve("u", `job:${i}`, 1_000_000 + i * 10_000);
        if (i % 3 !== 0) { const s = await l.settle(`job:${i}`, i * 77_777); charged += s.chargedMicro; }
      }
      const b = await l.balance("u");
      assert.equal(b.balanceMicro + b.reservedMicro + charged, topped);
    });

    test("heldReservations lists only stale unsettled holds", async () => {
      const l = await make();
      await l.topup("u", 5_000_000, "e");
      await l.reserve("u", "job:a", 1_000_000);
      await l.reserve("u", "job:b", 1_000_000);
      await l.settle("job:b", 0);
      assert.deepEqual((await l.heldReservations(0)).map((r) => r.key), ["job:a"]);
      assert.deepEqual(await l.heldReservations(3_600_000), []);
      assert.equal((await l.getReservation("job:b"))?.state, "settled");
      assert.equal(await l.getReservation("nope"), undefined);
    });

    test("amounts must be non-negative integers", async () => {
      const l = await make();
      await assert.rejects(l.topup("u", -1, "e"), RangeError);
      await assert.rejects(l.topup("u", 1.5, "e2"), RangeError);
      await assert.rejects(l.reserve("u", "k", -5), RangeError);
    });
  });
}

const job = (id: string, key: string, over: Partial<Job> = {}): Job => ({
  id, status: "queued", createdAt: Date.now(), reservedMicro: 1000, settled: false,
  input: { userId: "u", botName: "b", language: "python", entry: "bot.py", track: "ladder-v1", hands: 600, idempotencyKey: key, mode: "worker" }, ...over,
});

function storeContract(name: string, make: () => Promise<JobStore> | JobStore) {
  describe(`job store contract: ${name}`, () => {
    test("put is idempotent on the key; patch merges; queries", async () => {
      const s = await make();
      const a = await s.put(job("j1", "k1"));
      assert.equal(a.created, true);
      const dup = await s.put(job("j2", "k1"));
      assert.deepEqual([dup.created, dup.job.id], [false, "j1"]);
      await s.patch("j1", { status: "running", handsDone: 12 });
      assert.deepEqual([(await s.get("j1"))?.status, (await s.get("j1"))?.handsDone], ["running", 12]);
      assert.equal((await s.byKey("k1"))?.id, "j1");
      assert.equal(await s.get("zzz"), undefined);
      await s.put(job("j3", "k3", { status: "finished" }));
      await s.put(job("j4", "k4", { status: "failed", settled: true }));
      assert.deepEqual((await s.active()).map((j) => j.id), ["j1"]);
      assert.deepEqual((await s.unsettled()).map((j) => j.id), ["j3"]);
      await s.patch("j3", { settled: true });
      assert.deepEqual(await s.unsettled(), []);
      assert.equal((await s.listByUser("u", 10)).length, 3);
      assert.equal((await s.listByUser("nobody", 10)).length, 0);
      await assert.rejects(s.patch("zzz", { status: "failed" }), /no such job/);
    });
  });
}

ledgerContract("memory", () => new MemoryLedger());
storeContract("memory", () => new MemoryJobStore());

if (DB) {
  let admin: Pool;
  const pools: Pool[] = [];
  const fresh = async (): Promise<Pool> => {
    const schema = `t_${randomBytes(5).toString("hex")}`;
    await admin.query(`CREATE SCHEMA ${schema}`);
    const p = createPool(DB, { schema });
    pools.push(p);
    await migrate(p);
    return p;
  };
  before(() => { admin = createPool(DB); });
  after(async () => { for (const p of pools) await p.end(); await admin.end(); });
  ledgerContract("postgres", async () => new PgLedger(await fresh()));
  storeContract("postgres", async () => new PgJobStore(await fresh()));

  describe("postgres migrations", () => {
    test("migrate is idempotent and re-runnable concurrently", async () => {
      const p = await fresh();
      await Promise.all([migrate(p), migrate(p), migrate(p)]);
      const v = await p.query("SELECT version FROM runner_migrations");
      assert.equal(v.rowCount, 1);
    });
    test("the database itself refuses a negative balance", async () => {
      const p = await fresh();
      await p.query("INSERT INTO credit_accounts(user_id) VALUES ('u')");
      await assert.rejects(p.query("UPDATE credit_accounts SET balance_micro = -1 WHERE user_id = 'u'"), /check/i);
    });
  });
} else {
  test("postgres contract tests (set TEST_DATABASE_URL to run)", { skip: "TEST_DATABASE_URL not set" }, () => {});
}
