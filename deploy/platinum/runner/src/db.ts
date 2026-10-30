// Postgres plumbing: pool + forward-only migrations (own database/schema, separate from the arena's).
import pg from "pg";

export type Pool = pg.Pool;
export type Client = pg.PoolClient;

export function createPool(connectionString: string, opts: { schema?: string } = {}): Pool {
  // int8 (bigint) comes back as a JS number: micro-USD amounts stay far below 2^53.
  pg.types.setTypeParser(20, (v: string) => Number(v));
  return new pg.Pool({
    connectionString,
    max: Number(process.env.RUNNER_DB_POOL ?? 8),
    ...(opts.schema ? { options: `-c search_path=${opts.schema}` } : {}),
  });
}

export async function tx<T>(pool: Pool, fn: (c: Client) => Promise<T>): Promise<T> {
  const c = await pool.connect();
  try {
    await c.query("BEGIN");
    const out = await fn(c);
    await c.query("COMMIT");
    return out;
  } catch (e) {
    await c.query("ROLLBACK").catch(() => {});
    throw e;
  } finally {
    c.release();
  }
}

const MIGRATIONS: Array<{ version: number; name: string; sql: string }> = [
  {
    version: 1,
    name: "ledger-and-jobs",
    sql: `
      CREATE TABLE credit_accounts (
        user_id        text PRIMARY KEY,
        balance_micro  bigint NOT NULL DEFAULT 0 CHECK (balance_micro >= 0),   -- spendable
        reserved_micro bigint NOT NULL DEFAULT 0 CHECK (reserved_micro >= 0),  -- held for jobs in flight
        created_at     timestamptz NOT NULL DEFAULT now()
      );
      -- append-only audit trail; amount_micro is a magnitude, direction is implied by kind
      CREATE TABLE credit_entries (
        id              bigserial PRIMARY KEY,
        user_id         text NOT NULL REFERENCES credit_accounts(user_id),
        kind            text NOT NULL CHECK (kind IN ('topup','reserve','settle','release')),
        amount_micro    bigint NOT NULL CHECK (amount_micro >= 0),
        ref             text NOT NULL,
        idempotency_key text NOT NULL UNIQUE,
        meta            jsonb NOT NULL DEFAULT '{}'::jsonb,
        created_at      timestamptz NOT NULL DEFAULT now()
      );
      CREATE INDEX credit_entries_user ON credit_entries(user_id, id DESC);
      CREATE TABLE credit_reservations (
        key           text PRIMARY KEY,
        user_id       text NOT NULL REFERENCES credit_accounts(user_id),
        amount_micro  bigint NOT NULL CHECK (amount_micro >= 0),
        state         text NOT NULL CHECK (state IN ('held','settled')),
        charged_micro bigint,
        created_at    timestamptz NOT NULL DEFAULT now(),
        settled_at    timestamptz
      );
      CREATE INDEX credit_reservations_held ON credit_reservations(created_at) WHERE state = 'held';
      CREATE TABLE jobs (
        id              text PRIMARY KEY,
        idempotency_key text NOT NULL UNIQUE,
        user_id         text NOT NULL,
        status          text NOT NULL,
        settled         boolean NOT NULL DEFAULT false,
        doc             jsonb NOT NULL,
        created_at      timestamptz NOT NULL DEFAULT now(),
        updated_at      timestamptz NOT NULL DEFAULT now()
      );
      CREATE INDEX jobs_user ON jobs(user_id, created_at DESC);
      CREATE INDEX jobs_open ON jobs(status, settled);
    `,
  },
  {
    version: 2,
    name: "job_artifacts",
    sql: `
      CREATE TABLE job_artifacts (
        job_id     text PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
        tgz        bytea NOT NULL,
        created_at timestamptz NOT NULL DEFAULT now()
      );
    `,
  },
];

export async function migrate(pool: Pool): Promise<void> {
  const c = await pool.connect();
  try {
    await c.query("SELECT pg_advisory_lock(7351001)");
    await c.query("CREATE TABLE IF NOT EXISTS runner_migrations (version int PRIMARY KEY, name text NOT NULL, applied_at timestamptz NOT NULL DEFAULT now())");
    const done = new Set((await c.query("SELECT version FROM runner_migrations")).rows.map((r: { version: number }) => r.version));
    for (const m of MIGRATIONS) {
      if (done.has(m.version)) continue;
      await c.query("BEGIN");
      try {
        await c.query(m.sql);
        await c.query("INSERT INTO runner_migrations(version, name) VALUES ($1, $2)", [m.version, m.name]);
        await c.query("COMMIT");
      } catch (e) {
        await c.query("ROLLBACK");
        throw e;
      }
    }
  } finally {
    await c.query("SELECT pg_advisory_unlock(7351001)").catch(() => {});
    c.release();
  }
}
