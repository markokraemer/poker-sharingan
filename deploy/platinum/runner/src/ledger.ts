// Prepaid credit ledger: reserve-then-settle per job, idempotent on every write.
//
//   topup(user, amount, stripeEventId)   balance += amount                        (Stripe webhook; replays are no-ops)
//   reserve(user, jobKey, amount)        balance -= amount, reserved += amount    (before the job is accepted)
//   settle(jobKey, charge)               reserved -= amount, balance += amount - charge   (after teardown)
//
// Invariant per user: balance + reserved + (sum of settle charges) == (sum of top-ups). Accounts never go negative:
// reserve refuses (InsufficientCredits) instead. All amounts are integer micro-USD. Two implementations with one contract
// test suite: MemoryLedger (tests, RUNNER_STORAGE=memory) and PgLedger.
import type { Pool } from "./db.ts";
import { tx } from "./db.ts";

export class InsufficientCredits extends Error {
  balanceMicro: number;
  neededMicro: number;
  constructor(balanceMicro: number, neededMicro: number) {
    super(`insufficient credits: have ${balanceMicro} micro-USD available, need ${neededMicro}`);
    this.balanceMicro = balanceMicro;
    this.neededMicro = neededMicro;
  }
}

export class LedgerConflict extends Error {}

export interface Balance { userId: string; balanceMicro: number; reservedMicro: number }
export interface Reservation {
  key: string;
  userId: string;
  amountMicro: number;
  state: "held" | "settled";
  chargedMicro?: number;
  createdAt: number;
}
export interface LedgerEntry {
  id: number;
  userId: string;
  kind: "topup" | "reserve" | "settle" | "release";
  amountMicro: number;
  ref: string;
  idempotencyKey: string;
  meta: Record<string, unknown>;
  createdAt: number;
}
export interface SettleResult { settledNow: boolean; reservation: Reservation; chargedMicro: number; releasedMicro: number }

export interface Ledger {
  balance(userId: string): Promise<Balance>;
  topup(userId: string, amountMicro: number, eventId: string, meta?: Record<string, unknown>): Promise<{ applied: boolean; balance: Balance }>;
  /** Idempotent on `key`: a retry returns the existing reservation (`created:false`). Throws InsufficientCredits. */
  reserve(userId: string, key: string, amountMicro: number): Promise<{ created: boolean; reservation: Reservation }>;
  /** Idempotent on `key`. `chargeMicro` is clamped to [0, reserved]; the rest is released. */
  settle(key: string, chargeMicro: number, meta?: Record<string, unknown>): Promise<SettleResult>;
  getReservation(key: string): Promise<Reservation | undefined>;
  /** Reservations still held after `olderThanMs` (crash recovery). */
  heldReservations(olderThanMs: number): Promise<Reservation[]>;
  entries(userId: string, limit?: number): Promise<LedgerEntry[]>;
}

const checkAmount = (n: number, what: string) => {
  if (!Number.isSafeInteger(n) || n < 0) throw new RangeError(`${what} must be a non-negative integer (micro-USD), got ${n}`);
};

// ------------------------------------------------------------------------------------------------ memory

export class MemoryLedger implements Ledger {
  private accounts = new Map<string, { balance: number; reserved: number }>();
  private reservations = new Map<string, Reservation>();
  private log: LedgerEntry[] = [];
  private keys = new Set<string>();
  private now: () => number;

  constructor(now: () => number = Date.now) { this.now = now; }

  private acct(userId: string) {
    let a = this.accounts.get(userId);
    if (!a) { a = { balance: 0, reserved: 0 }; this.accounts.set(userId, a); }
    return a;
  }
  private add(userId: string, kind: LedgerEntry["kind"], amountMicro: number, ref: string, idempotencyKey: string, meta: Record<string, unknown>) {
    this.keys.add(idempotencyKey);
    this.log.push({ id: this.log.length + 1, userId, kind, amountMicro, ref, idempotencyKey, meta, createdAt: this.now() });
  }

  async balance(userId: string): Promise<Balance> {
    const a = this.accounts.get(userId) ?? { balance: 0, reserved: 0 };
    return { userId, balanceMicro: a.balance, reservedMicro: a.reserved };
  }

  async topup(userId: string, amountMicro: number, eventId: string, meta: Record<string, unknown> = {}) {
    checkAmount(amountMicro, "top-up");
    const key = `topup:${eventId}`;
    if (this.keys.has(key)) return { applied: false, balance: await this.balance(userId) };
    this.acct(userId).balance += amountMicro;
    this.add(userId, "topup", amountMicro, eventId, key, meta);
    return { applied: true, balance: await this.balance(userId) };
  }

  async reserve(userId: string, key: string, amountMicro: number) {
    checkAmount(amountMicro, "reservation");
    const existing = this.reservations.get(key);
    if (existing) {
      if (existing.userId !== userId || existing.amountMicro !== amountMicro) throw new LedgerConflict(`reservation ${key} exists with different parameters`);
      return { created: false, reservation: { ...existing } };
    }
    const a = this.acct(userId);
    if (a.balance < amountMicro) throw new InsufficientCredits(a.balance, amountMicro);
    a.balance -= amountMicro;
    a.reserved += amountMicro;
    const r: Reservation = { key, userId, amountMicro, state: "held", createdAt: this.now() };
    this.reservations.set(key, r);
    this.add(userId, "reserve", amountMicro, key, `reserve:${key}`, {});
    return { created: true, reservation: { ...r } };
  }

  async settle(key: string, chargeMicro: number, meta: Record<string, unknown> = {}): Promise<SettleResult> {
    checkAmount(chargeMicro, "charge");
    const r = this.reservations.get(key);
    if (!r) throw new LedgerConflict(`no such reservation ${key}`);
    if (r.state === "settled") {
      const charged = r.chargedMicro ?? 0;
      return { settledNow: false, reservation: { ...r }, chargedMicro: charged, releasedMicro: r.amountMicro - charged };
    }
    const charged = Math.min(chargeMicro, r.amountMicro);
    const released = r.amountMicro - charged;
    const a = this.acct(r.userId);
    a.reserved -= r.amountMicro;
    a.balance += released;
    r.state = "settled";
    r.chargedMicro = charged;
    this.add(r.userId, "settle", charged, key, `settle:${key}`, meta);
    if (released > 0) this.add(r.userId, "release", released, key, `release:${key}`, {});
    return { settledNow: true, reservation: { ...r }, chargedMicro: charged, releasedMicro: released };
  }

  async getReservation(key: string) { const r = this.reservations.get(key); return r ? { ...r } : undefined; }

  async heldReservations(olderThanMs: number) {
    const cutoff = this.now() - olderThanMs;
    return [...this.reservations.values()].filter((r) => r.state === "held" && r.createdAt <= cutoff).map((r) => ({ ...r }));
  }

  async entries(userId: string, limit = 50) {
    return this.log.filter((e) => e.userId === userId).slice(-limit).reverse();
  }
}

// ------------------------------------------------------------------------------------------------ postgres

interface ResRow { key: string; user_id: string; amount_micro: number; state: "held" | "settled"; charged_micro: number | null; created_at: Date }
const toReservation = (r: ResRow): Reservation => ({
  key: r.key, userId: r.user_id, amountMicro: r.amount_micro, state: r.state,
  chargedMicro: r.charged_micro ?? undefined, createdAt: r.created_at.getTime(),
});

export class PgLedger implements Ledger {
  private readonly pool: Pool;
  constructor(pool: Pool) { this.pool = pool; }

  async balance(userId: string): Promise<Balance> {
    const r = await this.pool.query("SELECT balance_micro, reserved_micro FROM credit_accounts WHERE user_id = $1", [userId]);
    const row = r.rows[0];
    return { userId, balanceMicro: row?.balance_micro ?? 0, reservedMicro: row?.reserved_micro ?? 0 };
  }

  async topup(userId: string, amountMicro: number, eventId: string, meta: Record<string, unknown> = {}) {
    checkAmount(amountMicro, "top-up");
    const applied = await tx(this.pool, async (c) => {
      await c.query("INSERT INTO credit_accounts(user_id) VALUES ($1) ON CONFLICT DO NOTHING", [userId]);
      const ins = await c.query(
        "INSERT INTO credit_entries(user_id, kind, amount_micro, ref, idempotency_key, meta) VALUES ($1,'topup',$2,$3,$4,$5) ON CONFLICT (idempotency_key) DO NOTHING RETURNING id",
        [userId, amountMicro, eventId, `topup:${eventId}`, JSON.stringify(meta)],
      );
      if (ins.rowCount === 0) return false;
      await c.query("UPDATE credit_accounts SET balance_micro = balance_micro + $2 WHERE user_id = $1", [userId, amountMicro]);
      return true;
    });
    return { applied, balance: await this.balance(userId) };
  }

  async reserve(userId: string, key: string, amountMicro: number) {
    checkAmount(amountMicro, "reservation");
    return tx(this.pool, async (c) => {
      await c.query("INSERT INTO credit_accounts(user_id) VALUES ($1) ON CONFLICT DO NOTHING", [userId]);
      const acct = (await c.query("SELECT balance_micro FROM credit_accounts WHERE user_id = $1 FOR UPDATE", [userId])).rows[0];
      const ex = (await c.query<ResRow>("SELECT * FROM credit_reservations WHERE key = $1", [key])).rows[0];
      if (ex) {
        if (ex.user_id !== userId || ex.amount_micro !== amountMicro) throw new LedgerConflict(`reservation ${key} exists with different parameters`);
        return { created: false, reservation: toReservation(ex) };
      }
      if (acct.balance_micro < amountMicro) throw new InsufficientCredits(acct.balance_micro, amountMicro);
      await c.query("UPDATE credit_accounts SET balance_micro = balance_micro - $2, reserved_micro = reserved_micro + $2 WHERE user_id = $1", [userId, amountMicro]);
      const row = (await c.query<ResRow>(
        "INSERT INTO credit_reservations(key, user_id, amount_micro, state) VALUES ($1,$2,$3,'held') RETURNING *", [key, userId, amountMicro],
      )).rows[0]!;
      await c.query(
        "INSERT INTO credit_entries(user_id, kind, amount_micro, ref, idempotency_key) VALUES ($1,'reserve',$2,$3,$4)", [userId, amountMicro, key, `reserve:${key}`],
      );
      return { created: true, reservation: toReservation(row) };
    });
  }

  async settle(key: string, chargeMicro: number, meta: Record<string, unknown> = {}): Promise<SettleResult> {
    checkAmount(chargeMicro, "charge");
    return tx(this.pool, async (c) => {
      // lock order everywhere: account, then reservation
      const peek = (await c.query<ResRow>("SELECT * FROM credit_reservations WHERE key = $1", [key])).rows[0];
      if (!peek) throw new LedgerConflict(`no such reservation ${key}`);
      await c.query("SELECT 1 FROM credit_accounts WHERE user_id = $1 FOR UPDATE", [peek.user_id]);
      const r = (await c.query<ResRow>("SELECT * FROM credit_reservations WHERE key = $1 FOR UPDATE", [key])).rows[0]!;
      if (r.state === "settled") {
        const charged = r.charged_micro ?? 0;
        return { settledNow: false, reservation: toReservation(r), chargedMicro: charged, releasedMicro: r.amount_micro - charged };
      }
      const charged = Math.min(chargeMicro, r.amount_micro);
      const released = r.amount_micro - charged;
      await c.query("UPDATE credit_accounts SET reserved_micro = reserved_micro - $2, balance_micro = balance_micro + $3 WHERE user_id = $1", [r.user_id, r.amount_micro, released]);
      const upd = (await c.query<ResRow>(
        "UPDATE credit_reservations SET state = 'settled', charged_micro = $2, settled_at = now() WHERE key = $1 RETURNING *", [key, charged],
      )).rows[0]!;
      await c.query(
        "INSERT INTO credit_entries(user_id, kind, amount_micro, ref, idempotency_key, meta) VALUES ($1,'settle',$2,$3,$4,$5)",
        [r.user_id, charged, key, `settle:${key}`, JSON.stringify(meta)],
      );
      if (released > 0) {
        await c.query(
          "INSERT INTO credit_entries(user_id, kind, amount_micro, ref, idempotency_key) VALUES ($1,'release',$2,$3,$4)", [r.user_id, released, key, `release:${key}`],
        );
      }
      return { settledNow: true, reservation: toReservation(upd), chargedMicro: charged, releasedMicro: released };
    });
  }

  async getReservation(key: string) {
    const r = (await this.pool.query<ResRow>("SELECT * FROM credit_reservations WHERE key = $1", [key])).rows[0];
    return r ? toReservation(r) : undefined;
  }

  async heldReservations(olderThanMs: number) {
    const r = await this.pool.query<ResRow>(
      "SELECT * FROM credit_reservations WHERE state = 'held' AND created_at <= now() - ($1::bigint * interval '1 millisecond') ORDER BY created_at", [olderThanMs],
    );
    return r.rows.map(toReservation);
  }

  async entries(userId: string, limit = 50) {
    const r = await this.pool.query(
      "SELECT id, user_id, kind, amount_micro, ref, idempotency_key, meta, created_at FROM credit_entries WHERE user_id = $1 ORDER BY id DESC LIMIT $2", [userId, limit],
    );
    return r.rows.map((e: { id: number; user_id: string; kind: LedgerEntry["kind"]; amount_micro: number; ref: string; idempotency_key: string; meta: Record<string, unknown>; created_at: Date }) => ({
      id: e.id, userId: e.user_id, kind: e.kind, amountMicro: e.amount_micro, ref: e.ref, idempotencyKey: e.idempotency_key, meta: e.meta, createdAt: e.created_at.getTime(),
    }));
  }
}
