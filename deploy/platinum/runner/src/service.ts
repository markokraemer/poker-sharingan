// The product layer: credits + jobs + Stripe. HTTP-free so it is testable; app.ts is the thin HTTP skin.
//
//   submit(req)      validate -> reserve credits (idempotent on idempotencyKey) -> store job -> run in the background
//   run(job)         concurrency slot -> executeJob (sandbox, play, teardown) -> settle credits
//   settleJob(id)    charge = price of the hands the arena says were played (0 when WE failed); the rest is released
//   recover()        after a crash: fail orphaned jobs, settle finished-but-unsettled ones, release dangling reservations
import { randomBytes } from "node:crypto";
import type { ArenaClient } from "./arena.ts";
import { ArchiveError, scanTgz } from "./archive.ts";
import type { Config, JobMode } from "./config.ts";
import { chargeMicro, reserveMicro, roundHands } from "./cost.ts";
import { executeJob, reapOrphans } from "./job.ts";
import { InsufficientCredits, LedgerConflict, type Ledger } from "./ledger.ts";
import type { Logger } from "./log.ts";
import { microToUsd, usdToMicro } from "./money.ts";
import type { PlatinumClient } from "./platinum/types.ts";
import {
  handleStripeEvent, planTopup, StripeSignatureError, TopupRequestError, verifyStripeSignature,
  type CheckoutGateway, type CheckoutSession, type EventOutcome, type StripeEvent, type TopupRequest,
} from "./stripe.ts";
import { TERMINAL, type Job, type JobInput, type JobStore } from "./store.ts";

export class ServiceError extends Error {
  status: number;
  code: string;
  details?: Record<string, unknown>;
  constructor(status: number, code: string, message: string, details?: Record<string, unknown>) {
    super(message);
    this.status = status;
    this.code = code;
    this.details = details;
  }
}

export interface ServiceDeps {
  cfg: Config;
  ledger: Ledger;
  store: JobStore;
  platinum: PlatinumClient;
  arena: ArenaClient;
  gateway: CheckoutGateway;
  log: Logger;
  sleep?: (ms: number, signal?: AbortSignal) => Promise<void>;
}

export interface SubmitRequest {
  userId?: unknown; botName?: unknown; language?: unknown; artifactTgzBase64?: unknown; entry?: unknown;
  requirementsFile?: unknown; track?: unknown; hands?: unknown; lanes?: unknown; idempotencyKey?: unknown; mode?: unknown;
  contact?: unknown; profile?: unknown;
}

const TRACKS = new Set(["ladder-v1", "sharingan-1"]);
const SAFE_PATH = /^[\w./-]{1,200}$/;
const SAFE_ID = /^[\w.:@+-]{1,128}$/;
const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

/** Optional free-text field: trimmed, max 200 characters, no control characters; absent / null / "" is undefined. */
function optText(v: unknown, name: string): string | undefined {
  if (v === undefined || v === null || v === "") return undefined;
  const t = typeof v === "string" ? v.trim() : undefined;
  if (!t || t.length > 200 || /[\u0000-\u001f\u007f]/.test(t)) throw new ServiceError(400, "bad_request", `${name}: a string, max 200 characters`);
  return t;
}

/** A public https profile link (scheme optional: "x.com/you" works; "@you" means X), normalized; undefined when it is not one. */
export function profileUrl(s: string): string | undefined {
  if (/^@\w{1,15}$/.test(s)) return `https://x.com/${s.slice(1)}`;
  let u: URL;
  try { u = new URL(/^[a-z][a-z0-9+.-]*:\/\//i.test(s) ? s : `https://${s}`); } catch { return undefined; }
  if (u.protocol !== "https:" && u.protocol !== "http:") return undefined;
  if (u.username || u.password || u.port) return undefined;
  const host = u.hostname.toLowerCase();
  if (!/^([a-z0-9-]+\.)+[a-z]{2,}$/.test(host) || host.endsWith(".local")) return undefined;   // a public name, not an IP
  u.protocol = "https:";
  return u.toString();
}

class Semaphore {
  private n: number;
  private waiters: Array<() => void> = [];
  constructor(n: number) { this.n = n; }
  async acquire(signal: AbortSignal): Promise<void> {
    if (this.n > 0) { this.n--; return; }
    await new Promise<void>((resolve, reject) => {
      const w = () => { signal.removeEventListener("abort", onAbort); resolve(); };
      const onAbort = () => { this.waiters = this.waiters.filter((x) => x !== w); reject(new Error("aborted")); };
      signal.addEventListener("abort", onAbort, { once: true });
      this.waiters.push(w);
    });
  }
  release(): void { const w = this.waiters.shift(); if (w) w(); else this.n++; }
}

export class Service {
  readonly d: ServiceDeps;
  private readonly slots: Semaphore;
  private readonly aborts = new Map<string, AbortController>();
  private readonly running = new Map<string, Promise<void>>();
  private reaper?: ReturnType<typeof setInterval>;

  constructor(d: ServiceDeps) {
    this.d = d;
    this.slots = new Semaphore(d.cfg.limits.maxConcurrentRuns);
  }

  // ------------------------------------------------------------------------------------------ credits

  /** FREE mode: what this player has used of the rolling-24h quota. */
  async quota(userId: string) {
    const b = this.d.cfg.billing;
    const jobs = await this.d.store.listByUser(userId, 1000);
    const since = Date.now() - 24 * 3600_000;
    const used = jobs.filter((j) => j.createdAt >= since && !(j.status === "failed" && j.errorClass !== "bot"))
      .reduce((n, j) => n + (TERMINAL.has(j.status) ? (j.handsDone ?? 0) : j.input.hands), 0);
    return { user_id: userId, billing: b.mode, hands_per_day_limit: b.freeMaxHandsPerDay, hands_used_24h: used,
      concurrent_limit: b.freeMaxConcurrent, concurrent_active: jobs.filter((j) => !TERMINAL.has(j.status)).length };
  }

  async credits(userId: string) {
    if (!SAFE_ID.test(userId)) throw new ServiceError(400, "bad_request", "bad userId");
    if (this.d.cfg.billing.mode === "free") return { ...(await this.quota(userId)), balance_micro: 0, balance_usd: 0, reserved_micro: 0, reserved_usd: 0, price_usd_per_1k_hands: 0 };
    const b = await this.d.ledger.balance(userId);
    return {
      user_id: userId,
      balance_micro: b.balanceMicro, balance_usd: microToUsd(b.balanceMicro),
      reserved_micro: b.reservedMicro, reserved_usd: microToUsd(b.reservedMicro),
      price_usd_per_1k_hands: this.d.cfg.pricing.usdPerKHands,
    };
  }

  async entries(userId: string, limit = 50) {
    return (await this.d.ledger.entries(userId, Math.min(Math.max(limit, 1), 200))).map((e) => ({
      id: e.id, kind: e.kind, amount_micro: e.amountMicro, amount_usd: microToUsd(e.amountMicro), ref: e.ref, created_at: new Date(e.createdAt).toISOString(), meta: e.meta,
    }));
  }

  async checkout(req: TopupRequest): Promise<CheckoutSession> {
    if (this.d.cfg.billing.mode === "free") throw new ServiceError(400, "billing_disabled", "runs are free right now; there is nothing to buy");
    let plan;
    try { plan = planTopup(this.d.cfg.stripe, req); } catch (e) {
      if (e instanceof TopupRequestError) throw new ServiceError(400, "bad_request", e.message);
      throw e;
    }
    const base = this.d.cfg.stripe.publicBaseUrl;
    const urls = { successUrl: req.successUrl ?? `${base}/credits?checkout=success`, cancelUrl: req.cancelUrl ?? `${base}/credits?checkout=cancelled` };
    const s = await this.d.gateway.createSession(plan, urls, `ckt_${plan.userId}_${randomBytes(8).toString("hex")}`);
    this.d.log("checkout_created", { user: plan.userId, session: s.id, price_cents: plan.priceCents, simulated: s.simulated });
    return s;
  }

  /** Stripe webhook: verify the signature over the RAW body, then credit. Never makes a network call. */
  async stripeWebhook(rawBody: Buffer, signature: string | undefined): Promise<EventOutcome> {
    if (this.d.cfg.billing.mode === "free" && !this.d.cfg.stripe.webhookSecret) throw new ServiceError(404, "not_found", "billing is disabled");
    try {
      verifyStripeSignature(rawBody, signature, this.d.cfg.stripe.webhookSecret, { toleranceS: this.d.cfg.stripe.toleranceS });
    } catch (e) {
      if (e instanceof StripeSignatureError) throw new ServiceError(400, "bad_signature", e.message);
      throw e;
    }
    let event: StripeEvent;
    try { event = JSON.parse(rawBody.toString("utf8")); } catch { throw new ServiceError(400, "bad_request", "body is not JSON"); }
    if (!event?.id || !event?.type) throw new ServiceError(400, "bad_request", "not a Stripe event");
    const out = await handleStripeEvent(event, this.d.ledger);
    this.d.log("stripe_event", { id: event.id, type: event.type, ...out });
    return out;
  }

  // ------------------------------------------------------------------------------------------ quoting

  quote(hands: number) {
    const p = this.d.cfg.pricing;
    const n = roundHands(hands);
    if (this.d.cfg.billing.mode === "free") {
      return { hands: n, price_micro: 0, price_usd: 0, price_usd_per_1k_hands: 0, billing: "free", note: `free: up to ${this.d.cfg.billing.freeMaxHandsPerDay} hands per rolling 24 h, ${this.d.cfg.billing.freeMaxConcurrent} concurrent jobs per player` };
    }
    const reserve = reserveMicro(p, n);
    return { hands: n, price_micro: reserve, price_usd: microToUsd(reserve), price_usd_per_1k_hands: p.usdPerKHands, note: "charged by hands actually played; the unused part of the hold is released" };
  }

  // ------------------------------------------------------------------------------------------ jobs

  /**
   * Everything that can be decided from the request alone, cheapest first. Runs BEFORE the quota / credit checks, so garbage
   * is a 400 and never a 429 / 402 (and never costs a reservation).
   */
  private async validate(req: SubmitRequest, requireOwner: boolean): Promise<{ input: JobInput; artifact: Buffer }> {
    const cfg = this.d.cfg;

    function bad(m: string): never { throw new ServiceError(400, "bad_request", m); }
    const str = (k: keyof SubmitRequest, max = 200): string => {
      const v = req[k];
      return typeof v === "string" && v.length > 0 && v.length <= max ? v : bad(`${k} required (string, max ${max})`);
    };
    const userId = str("userId", 128);
    if (!SAFE_ID.test(userId)) bad("userId has unsupported characters");
    const botName = str("botName", 48);
    if (!/^[\w .-]+$/.test(botName)) bad("botName has unsupported characters");
    const language = req.language;
    if (language !== "python" && language !== "typescript") bad("language must be python|typescript");
    const entry = str("entry");
    if (!SAFE_PATH.test(entry) || entry.includes("..") || entry.startsWith("/")) bad("bad entry path");
    let requirementsFile: string | undefined;
    if (req.requirementsFile !== undefined && req.requirementsFile !== null && req.requirementsFile !== "") {
      requirementsFile = str("requirementsFile");
      if (!SAFE_PATH.test(requirementsFile) || requirementsFile.includes("..") || requirementsFile.startsWith("/")) bad("bad requirementsFile");
    }
    const track = str("track", 40);
    if (!TRACKS.has(track)) bad("unknown track");
    const hands = req.hands === undefined ? cfg.limits.defaultHands : Number(req.hands);
    if (!Number.isInteger(hands) || hands < cfg.limits.minHands || hands > cfg.limits.maxHands) bad(`hands must be an integer in ${cfg.limits.minHands}..${cfg.limits.maxHands}`);
    let lanes: number | undefined;
    if (req.lanes !== undefined) {
      lanes = Number(req.lanes);
      if (!Number.isInteger(lanes) || lanes < 1 || lanes > cfg.limits.maxLanes) bad(`lanes must be an integer in 1..${cfg.limits.maxLanes}`);
    }
    const idempotencyKey = str("idempotencyKey");
    const mode = (req.mode ?? cfg.worker.defaultMode) as JobMode;
    if (mode !== "worker" && mode !== "connect") bad("mode must be worker|connect");
    if (mode === "worker" && language === "typescript" && !cfg.worker.pipeCmd.typescript) bad("worker mode does not support typescript bots yet; use mode:\"connect\"");

    // who is behind the bot: an email to reach them + a public profile link (X, LinkedIn, GitHub, a personal site...) that
    // ties the bot to a real person. Required on every public submit; optional for our own service-token submits.
    const contact = optText(req.contact, "contact");
    const profile = optText(req.profile, "profile");
    if (requireOwner && (!contact || !profile)) {
      bad("contact and profile required: an email (--contact you@example.com) and a link to your X, LinkedIn, GitHub or personal page "
        + "(--profile https://x.com/you); never shown publicly. Old CLI? pip install -U \"git+https://github.com/markokraemer/poker-sharingan#subdirectory=sdk/python\"");
    }
    if (contact !== undefined && !EMAIL.test(contact)) bad("contact must be an email address");
    if (profile !== undefined && !profileUrl(profile)) bad("profile must be a link to your X, LinkedIn, GitHub or personal page, like https://x.com/you");

    const b64 = str("artifactTgzBase64", Math.ceil(cfg.limits.maxArtifactBytes * 1.4) + 1024);
    const compact = b64.replace(/\s+/g, "");
    if (compact.length === 0 || compact.length % 4 !== 0 || !/^[A-Za-z0-9+/]+={0,2}$/.test(compact)) bad("artifactTgzBase64 is not valid base64");
    const artifact = Buffer.from(compact, "base64");
    if (artifact.length === 0) bad("artifactTgzBase64 is empty");
    if (artifact.length > cfg.limits.maxArtifactBytes) bad(`artifact too large (max ${cfg.limits.maxArtifactBytes} bytes)`);
    // a gzip tarball that actually contains the entry file (and the requirements file, if named); stops early, never unpacks
    try {
      const scan = await scanTgz(artifact, [entry, ...(requirementsFile ? [requirementsFile] : [])]);
      if (scan.missing.includes(entry.replace(/^(\.\/)+/, ""))) bad(`entry file ${entry} not found in the archive`);
      if (requirementsFile && scan.missing.includes(requirementsFile.replace(/^(\.\/)+/, ""))) bad(`requirements file ${requirementsFile} not found in the archive`);
    } catch (e) {
      if (e instanceof ArchiveError) bad(e.message);
      throw e;
    }
    // hands are echoed back rounded up to whole deals (6 hands), so the job says what is actually played
    return { input: { userId, botName, language, entry, requirementsFile, track, hands: roundHands(hands), lanes, idempotencyKey, mode, ...(contact ? { contact } : {}), ...(profile ? { profile: profileUrl(profile)! } : {}) }, artifact };
  }

  private readonly submitLock = { p: Promise.resolve() as Promise<unknown> };

  /** FREE mode: per-player quotas (rolling 24 h hands, concurrent jobs). Called under the submit lock. */
  private async checkFreeQuota(userId: string, hands: number, clientIp?: string): Promise<void> {
    const b = this.d.cfg.billing;
    if (clientIp && b.freeMaxConcurrentPerIp > 0) {
      const fromIp = (await this.d.store.active()).filter((j) => j.input.clientIp === clientIp).length;
      if (fromIp >= b.freeMaxConcurrentPerIp) {
        throw new ServiceError(429, "quota_concurrent_ip", `at most ${b.freeMaxConcurrentPerIp} jobs at once from one address (all your bots together); wait for one to finish`,
          { limit: b.freeMaxConcurrentPerIp, active: fromIp });
      }
    }
    const jobs = await this.d.store.listByUser(userId, 1000);
    const since = Date.now() - 24 * 3600_000;
    const active = jobs.filter((j) => !TERMINAL.has(j.status)).length;
    if (active >= b.freeMaxConcurrent) {
      throw new ServiceError(429, "quota_concurrent", `at most ${b.freeMaxConcurrent} jobs at once; wait for one to finish`, { limit: b.freeMaxConcurrent, active });
    }
    const used = jobs.filter((j) => j.createdAt >= since && !(j.status === "failed" && j.errorClass !== "bot"))
      .reduce((n, j) => n + (TERMINAL.has(j.status) ? (j.handsDone ?? 0) : j.input.hands), 0);
    if (used + roundHands(hands) > b.freeMaxHandsPerDay) {
      throw new ServiceError(429, "quota_hands_per_day", `daily quota of ${b.freeMaxHandsPerDay} hands (rolling 24 h) would be exceeded: used ${used}, requested ${roundHands(hands)}`,
        { limit: b.freeMaxHandsPerDay, used, requested: roundHands(hands) });
    }
  }

  /**
   * `asUser`: set for a public player authenticated with an arena bot token. The job is forced onto that player and its
   * idempotency key is namespaced, so one player can never read or dedupe into another's jobs.
   */
  async submit(req: SubmitRequest, asUser?: string, clientIp?: string): Promise<{ job: Job; deduped: boolean }> {
    // snake_case is accepted too (the rest of the API is snake_case); camelCase stays canonical for existing clients.
    // Done first, so a snake_case idempotency key is namespaced per player like a camelCase one.
    const r = { ...(req as Record<string, unknown>) };
    for (const [snake, camel] of [["user_id", "userId"], ["bot_name", "botName"], ["artifact_tgz_base64", "artifactTgzBase64"],
      ["requirements_file", "requirementsFile"], ["idempotency_key", "idempotencyKey"]] as const) {
      if (r[camel] === undefined && r[snake] !== undefined) r[camel] = r[snake];
    }
    req = r as SubmitRequest;
    // public players only get worker mode (no network at all); connect mode needs the sandbox on the arena's network
    if (asUser) req = { ...req, mode: "worker", userId: asUser, idempotencyKey: typeof req.idempotencyKey === "string" ? `${asUser}:${req.idempotencyKey}`.slice(0, 200) : req.idempotencyKey };
    const { input, artifact } = await this.validate(req, !!asUser);
    if (asUser) input.playerId = asUser;
    if (asUser && clientIp) input.clientIp = clientIp.slice(0, 64);
    const existing = await this.d.store.byKey(input.idempotencyKey);
    if (existing) {
      const a = existing.input, same = a.track === input.track && a.hands === input.hands && a.botName === input.botName && a.entry === input.entry;
      if (!same) {
        this.d.log("idempotency_conflict", { key: input.idempotencyKey, was: [a.track, a.hands, a.botName, a.entry], now: [input.track, input.hands, input.botName, input.entry] });
        throw new ServiceError(409, "idempotency_conflict", "this idempotencyKey was already used for a different job; use a new key");
      }
      return { job: existing, deduped: true };
    }

    if (this.d.cfg.billing.mode === "free") {
      // serialise quota check + insert so two parallel submits cannot both pass the check
      const run = this.submitLock.p.then(async () => {
        await this.checkFreeQuota(input.userId, input.hands, input.clientIp);
        const job: Job = { id: `job_${randomBytes(8).toString("hex")}`, input, status: "queued", createdAt: Date.now(), reservedMicro: 0, settled: false };
        const put = await this.d.store.put(job);
        return put;
      });
      this.submitLock.p = run.catch(() => {});
      const put = await run;
      if (!put.created) return { job: put.job, deduped: true };
      const job = put.job;
      await this.keepArtifact(job.id, artifact);
      this.d.log("job_accepted", { job: job.id, user: input.userId, track: input.track, hands: input.hands, mode: input.mode, billing: "free" });
      const p = this.run(job.id, artifact).catch((e) => this.d.log("job_run_crashed", { job: job.id, error: String(e) }));
      this.running.set(job.id, p);
      void p.finally(() => this.running.delete(job.id));
      return { job, deduped: false };
    }

    const amount = reserveMicro(this.d.cfg.pricing, input.hands);
    try {
      await this.d.ledger.reserve(input.userId, `job:${input.idempotencyKey}`, amount);
    } catch (e) {
      if (e instanceof InsufficientCredits) {
        throw new ServiceError(402, "insufficient_credits", `this run costs up to ${microToUsd(e.neededMicro).toFixed(4)} USD, balance is ${microToUsd(e.balanceMicro).toFixed(4)} USD`,
          { needed_micro: e.neededMicro, balance_micro: e.balanceMicro });
      }
      if (e instanceof LedgerConflict) throw new ServiceError(409, "idempotency_conflict", "idempotencyKey was used with different parameters");
      throw e;
    }
    const job: Job = { id: `job_${randomBytes(8).toString("hex")}`, input, status: "queued", createdAt: Date.now(), reservedMicro: amount, settled: false };
    const put = await this.d.store.put(job);
    if (!put.created) return { job: put.job, deduped: true };
    await this.keepArtifact(job.id, artifact);
    this.d.log("job_accepted", { job: job.id, user: input.userId, track: input.track, hands: input.hands, mode: input.mode, reserved_micro: amount });
    const p = this.run(job.id, artifact).catch((e) => this.d.log("job_run_crashed", { job: job.id, error: String(e) }));
    this.running.set(job.id, p);
    void p.finally(() => this.running.delete(job.id));
    return { job, deduped: false };
  }

  /** Keep the uploaded bot so the organizers can re-run it later. A storage hiccup is logged, never fails the job. */
  private async keepArtifact(jobId: string, artifact: Uint8Array): Promise<void> {
    try { await this.d.store.putArtifact(jobId, artifact); } catch (e) { this.d.log("artifact_store_failed", { job: jobId, error: String(e) }); }
  }

  async getArtifact(jobId: string): Promise<Uint8Array | undefined> { return this.d.store.getArtifact(jobId); }

  private async run(jobId: string, artifact: Uint8Array): Promise<void> {
    const ac = new AbortController();
    this.aborts.set(jobId, ac);
    let acquired = false;
    try {
      try {
        await this.slots.acquire(ac.signal);
        acquired = true;
      } catch {
        await this.d.store.patch(jobId, { status: "aborted", error: "aborted", finishedAt: Date.now() });
        return;
      }
      await executeJob({ cfg: this.d.cfg, platinum: this.d.platinum, arena: this.d.arena, store: this.d.store, log: this.d.log, sleep: this.d.sleep }, jobId, artifact, ac);
    } catch (e) {
      this.d.log("job_crashed", { job: jobId, error: String(e) });
      const j = await this.d.store.get(jobId);
      if (j && !TERMINAL.has(j.status)) await this.d.store.patch(jobId, { status: "failed", errorClass: "infra", error: `runner error: ${String(e).slice(0, 300)}`, finishedAt: Date.now() });
    } finally {
      if (acquired) this.slots.release();
      this.aborts.delete(jobId);
      await this.settleJob(jobId).catch((e) => this.d.log("settle_failed", { job: jobId, error: String(e) }));
    }
  }

  /** What this job costs the player. Pure policy: hands played x price, unless WE failed. */
  chargeFor(job: Job): number {
    if (this.d.cfg.billing.mode === "free") return 0;
    const p = this.d.cfg.pricing;
    if (job.status === "failed" && job.errorClass !== "bot") return 0;           // our fault (or unknown): refund in full
    const hands = job.handsDone ?? 0;
    if (hands > 0) return chargeMicro(p, hands);
    if (job.status === "failed" && job.errorClass === "bot") return usdToMicro(p.failedBotChargeUsd);
    return 0;
  }

  async settleJob(jobId: string): Promise<void> {
    const job = await this.d.store.get(jobId);
    if (!job || !TERMINAL.has(job.status) || job.settled) return;
    const charge = this.chargeFor(job);
    if (this.d.cfg.billing.mode === "free") {
      await this.d.store.patch(jobId, { settled: true, chargeMicro: 0, releasedMicro: 0 });
      this.d.log("job_settled", { job: jobId, status: job.status, hands: job.handsDone ?? 0, charge_micro: 0, billing: "free", cogs_micro: job.cogsMicro, sandbox_deleted: job.sandboxDeleted });
      return;
    }
    const r = await this.d.ledger.settle(`job:${job.input.idempotencyKey}`, charge, { job: job.id, status: job.status, hands: job.handsDone ?? 0, error_class: job.errorClass });
    await this.d.store.patch(jobId, { settled: true, chargeMicro: r.chargedMicro, releasedMicro: r.releasedMicro });
    this.d.log("job_settled", { job: jobId, status: job.status, hands: job.handsDone ?? 0, charge_micro: r.chargedMicro, released_micro: r.releasedMicro, cogs_micro: job.cogsMicro, sandbox_deleted: job.sandboxDeleted });
  }

  async getJob(id: string): Promise<Job | undefined> { return this.d.store.get(id); }
  async listJobs(userId: string, limit = 20): Promise<Job[]> { return this.d.store.listByUser(userId, Math.min(Math.max(limit, 1), 100)); }

  async abort(id: string): Promise<boolean> {
    const job = await this.d.store.get(id);
    if (!job) throw new ServiceError(404, "not_found", "no such job");
    if (TERMINAL.has(job.status)) return false;
    this.aborts.get(id)?.abort();
    return true;
  }

  // ------------------------------------------------------------------------------------------ recovery / lifecycle

  async recover(): Promise<{ failed: number; settled: number; released: number; reaped: number }> {
    const { store, ledger, platinum, log } = this.d;
    let failed = 0, settled = 0, released = 0;
    for (const j of await store.active()) {
      if (this.aborts.has(j.id)) continue;                 // running in this process
      if (j.sandboxId) await platinum.deleteSandbox(j.sandboxId).catch(() => {});
      await store.patch(j.id, { status: "failed", errorClass: "infra", error: "runner restarted while the job was running", finishedAt: Date.now(), sandboxDeleted: true });
      failed++;
    }
    for (const j of await store.unsettled()) {
      if (this.aborts.has(j.id)) continue;
      await this.settleJob(j.id);
      settled++;
    }
    // A reservation with no job: crash between reserve and store.put. Give it back once it is clearly stale.
    const stale = this.d.cfg.limits.runDeadlineS * 1000 + 10 * 60_000;
    for (const r of await ledger.heldReservations(stale)) {
      if (!r.key.startsWith("job:")) continue;
      if (await store.byKey(r.key.slice(4))) continue;
      await ledger.settle(r.key, 0, { reason: "orphaned reservation" });
      released++;
    }
    const reaped = await reapOrphans({ platinum, store, log }).catch(() => 0);
    log("recovered", { failed, settled, released, reaped });
    return { failed, settled, released, reaped };
  }

  start(): void {
    this.reaper = setInterval(() => void reapOrphans({ platinum: this.d.platinum, store: this.d.store, log: this.d.log }).catch((e) => this.d.log("reap_failed", { error: String(e) })), 5 * 60_000);
    this.reaper.unref();
  }

  /** Wait for all in-flight jobs (tests, graceful shutdown). */
  async drain(): Promise<void> { await Promise.allSettled([...this.running.values()]); }
  stop(): void { if (this.reaper) clearInterval(this.reaper); }
  inFlight(): number { return this.running.size; }
}
