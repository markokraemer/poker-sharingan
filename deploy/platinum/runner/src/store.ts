// Job store: Postgres (jobs survive a runner restart) or in-memory (tests). A job is one JSON document plus a few indexed
// columns; patches are shallow merges.
import type { JobMode } from "./config.ts";
import type { Pool } from "./db.ts";

export type JobStatus = "queued" | "provisioning" | "connecting" | "running" | "finalizing" | "finished" | "failed" | "aborted";
export const TERMINAL: ReadonlySet<JobStatus> = new Set(["finished", "failed", "aborted"]);

/** Who is at fault when a job fails. Decides the charge: infra => refund in full; bot => charged for hands played. */
export type ErrorClass = "infra" | "bot";

export interface JobInput {
  userId: string;
  botName: string;
  language: "python" | "typescript";
  /** Path of the bot's entry file inside the archive, e.g. "bot.py". */
  entry: string;
  requirementsFile?: string;     // python: needs PyPI egress during setup
  track: string;                 // "ladder-v1" | "sharingan-1"
  hands: number;                 // rounded up to a multiple of 6
  lanes?: number;
  idempotencyKey: string;        // one job per key; the credit reservation is keyed by it too
  mode: JobMode;
  /** Set when a public player submits with their own arena token: the run is credited to that player (worker mode). */
  playerId?: string;
  /** Email of the bot's owner, so the organizers can reach them (competition prizes). Required on public submits. Never shown publicly. */
  contact?: string;
  /** Link to the owner's public profile (X, LinkedIn, GitHub, a personal page), normalized to https. Required on public submits. Never shown publicly. */
  profile?: string;
  /** Public submits: the client address (from our proxy), for the per-address concurrency cap. Never shown publicly. */
  clientIp?: string;
  /** sha256 of the uploaded .tar.gz: the identity of the bot that set a score (shown to its owner, kept with the upload). */
  artifactSha256?: string;
}

export interface Job {
  id: string;
  input: JobInput;
  status: JobStatus;
  error?: string;
  errorClass?: ErrorClass;
  createdAt: number;
  updatedAt?: number;
  finishedAt?: number;
  sandboxId?: string;
  sandboxDeleted?: boolean;
  arenaPlayerId?: string;
  runId?: string;
  handsDone?: number;
  handsTotal?: number;
  runningMbb?: number | null;
  mbbPerGame?: number | null;
  ci95Mbb?: number | null;
  verified?: boolean;
  /** worker mode: hands the bot forfeited (crash / timeouts); still dealt, scored and billed. */
  forfeitHands?: number;
  sandboxSeconds?: number;
  platinumUsd?: number;
  cogsMicro?: number;
  reservedMicro: number;
  chargeMicro?: number;
  releasedMicro?: number;
  settled: boolean;
  botLogTail?: string;
  resultAck?: Record<string, unknown>;
  /** worker mode: the secret deal seed (`run:<64 hex>`) of this run. Set only when the job is over; never shown before. */
  dealsSeed?: string;
  /** The bot's code started running (the worker was started). A failure after this costs quota even when it is ours. */
  ranBot?: boolean;
}

export interface JobStore {
  get(id: string): Promise<Job | undefined>;
  byKey(key: string): Promise<Job | undefined>;
  /** Insert; if the idempotency key already exists, returns the existing job and `created:false`. */
  put(job: Job): Promise<{ job: Job; created: boolean }>;
  patch(id: string, p: Partial<Job>): Promise<Job>;
  /** Not yet terminal. */
  active(): Promise<Job[]>;
  /** Terminal but credits not settled yet (crash between teardown and settle). */
  unsettled(): Promise<Job[]>;
  listByUser(userId: string, limit: number): Promise<Job[]>;
  /** Jobs created since `since` (ms) on `track` whose owner has this profile or this contact (any bot of that person). */
  listByOwner(track: string, profile: string | undefined, contact: string | undefined, since: number): Promise<Job[]>;
  /** The uploaded bot archive of a job, kept so the organizers can re-run the bot (competition final run). */
  putArtifact(id: string, tgz: Uint8Array): Promise<void>;
  getArtifact(id: string): Promise<Uint8Array | undefined>;
}

export class MemoryJobStore implements JobStore {
  private jobs = new Map<string, Job>();
  private artifacts = new Map<string, Uint8Array>();

  async putArtifact(id: string, tgz: Uint8Array) { if (!this.artifacts.has(id)) this.artifacts.set(id, new Uint8Array(tgz)); }
  async getArtifact(id: string) { const a = this.artifacts.get(id); return a ? new Uint8Array(a) : undefined; }

  async get(id: string) { const j = this.jobs.get(id); return j ? structuredClone(j) : undefined; }
  async byKey(key: string) {
    for (const j of this.jobs.values()) if (j.input.idempotencyKey === key) return structuredClone(j);
    return undefined;
  }
  async put(job: Job) {
    const ex = await this.byKey(job.input.idempotencyKey);
    if (ex) return { job: ex, created: false };
    this.jobs.set(job.id, structuredClone(job));
    return { job: structuredClone(job), created: true };
  }
  async patch(id: string, p: Partial<Job>) {
    const j = this.jobs.get(id);
    if (!j) throw new Error(`no such job ${id}`);
    Object.assign(j, p, { updatedAt: Date.now() });
    return structuredClone(j);
  }
  async active() { return [...this.jobs.values()].filter((j) => !TERMINAL.has(j.status)).map((j) => structuredClone(j)); }
  async unsettled() { return [...this.jobs.values()].filter((j) => TERMINAL.has(j.status) && !j.settled).map((j) => structuredClone(j)); }
  async listByUser(userId: string, limit: number) {
    return [...this.jobs.values()].filter((j) => j.input.userId === userId).sort((a, b) => b.createdAt - a.createdAt).slice(0, limit).map((j) => structuredClone(j));
  }
  async listByOwner(track: string, profile: string | undefined, contact: string | undefined, since: number) {
    const c = contact?.toLowerCase();
    return [...this.jobs.values()].filter((j) => j.input.track === track && j.createdAt >= since
      && ((profile !== undefined && j.input.profile === profile) || (c !== undefined && j.input.contact?.toLowerCase() === c))).map((j) => structuredClone(j));
  }
}

export class PgJobStore implements JobStore {
  private readonly pool: Pool;
  constructor(pool: Pool) { this.pool = pool; }

  async get(id: string) {
    const r = await this.pool.query("SELECT doc FROM jobs WHERE id = $1", [id]);
    return r.rows[0]?.doc as Job | undefined;
  }
  async byKey(key: string) {
    const r = await this.pool.query("SELECT doc FROM jobs WHERE idempotency_key = $1", [key]);
    return r.rows[0]?.doc as Job | undefined;
  }
  async put(job: Job) {
    const ins = await this.pool.query(
      "INSERT INTO jobs(id, idempotency_key, user_id, status, settled, doc) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT (idempotency_key) DO NOTHING RETURNING doc",
      [job.id, job.input.idempotencyKey, job.input.userId, job.status, job.settled, JSON.stringify(job)],
    );
    if (ins.rowCount) return { job: ins.rows[0].doc as Job, created: true };
    return { job: (await this.byKey(job.input.idempotencyKey))!, created: false };
  }
  async patch(id: string, p: Partial<Job>) {
    const patch = { ...p, updatedAt: Date.now() };
    const r = await this.pool.query(
      `UPDATE jobs SET doc = doc || $2::jsonb,
         status = COALESCE($2::jsonb->>'status', status),
         settled = COALESCE(($2::jsonb->>'settled')::boolean, settled),
         updated_at = now()
       WHERE id = $1 RETURNING doc`,
      [id, JSON.stringify(patch)],
    );
    if (!r.rowCount) throw new Error(`no such job ${id}`);
    return r.rows[0].doc as Job;
  }
  async active() {
    return (await this.pool.query("SELECT doc FROM jobs WHERE status NOT IN ('finished','failed','aborted')")).rows.map((r: { doc: Job }) => r.doc);
  }
  async unsettled() {
    return (await this.pool.query("SELECT doc FROM jobs WHERE status IN ('finished','failed','aborted') AND NOT settled")).rows.map((r: { doc: Job }) => r.doc);
  }
  async listByUser(userId: string, limit: number) {
    return (await this.pool.query("SELECT doc FROM jobs WHERE user_id = $1 ORDER BY created_at DESC LIMIT $2", [userId, limit])).rows.map((r: { doc: Job }) => r.doc);
  }
  async listByOwner(track: string, profile: string | undefined, contact: string | undefined, since: number) {
    return (await this.pool.query(
      `SELECT doc FROM jobs WHERE created_at >= to_timestamp($2::double precision / 1000) AND doc->'input'->>'track' = $1
         AND ((doc->'input'->>'profile') = $3 OR lower(doc->'input'->>'contact') = lower($4))`,
      [track, since, profile ?? null, contact ?? null],
    )).rows.map((r: { doc: Job }) => r.doc);
  }
  async putArtifact(id: string, tgz: Uint8Array) {
    await this.pool.query("INSERT INTO job_artifacts(job_id, tgz) VALUES ($1, $2) ON CONFLICT (job_id) DO NOTHING", [id, Buffer.from(tgz)]);
  }
  async getArtifact(id: string) {
    const r = await this.pool.query("SELECT tgz FROM job_artifacts WHERE job_id = $1", [id]);
    return r.rows[0] ? new Uint8Array(r.rows[0].tgz as Buffer) : undefined;
  }
}
