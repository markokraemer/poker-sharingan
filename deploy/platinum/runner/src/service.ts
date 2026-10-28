// The product layer: credits + jobs + Stripe. HTTP-free so it is testable; app.ts is the thin HTTP skin.
//
//   submit(req)      validate -> reserve credits (idempotent on idempotencyKey) -> store job -> run in the background
//   run(job)         concurrency slot -> executeJob (sandbox, play, teardown) -> settle credits
//   settleJob(id)    charge = price of the hands the arena says were played (0 when WE failed); the rest is released
//   recover()        after a crash: fail orphaned jobs, settle finished-but-unsettled ones, release dangling reservations
import { randomBytes } from "node:crypto";
import type { ArenaClient } from "./arena.ts";
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
}

const TRACKS = new Set(["ladder-v1", "sharingan-1"]);
const SAFE_PATH = /^[\w./-]{1,200}$/;
const SAFE_ID = /^[\w.:@+-]{1,128}$/;

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

  async credits(userId: string) {
    if (!SAFE_ID.test(userId)) throw new ServiceError(400, "bad_request", "bad userId");
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
    const reserve = reserveMicro(p, n);
    return { hands: n, price_micro: reserve, price_usd: microToUsd(reserve), price_usd_per_1k_hands: p.usdPerKHands, note: "charged by hands actually played; the unused part of the hold is released" };
  }

  // ------------------------------------------------------------------------------------------ jobs

  private validate(req: SubmitRequest): { input: JobInput; artifact: Buffer } {
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

    const b64 = str("artifactTgzBase64", Math.ceil(cfg.limits.maxArtifactBytes * 1.4) + 1024);
    const artifact = Buffer.from(b64, "base64");
    if (artifact.length === 0) bad("artifactTgzBase64 is empty");
    if (artifact.length > cfg.limits.maxArtifactBytes) bad(`artifact too large (max ${cfg.limits.maxArtifactBytes} bytes)`);
    return { input: { userId, botName, language, entry, requirementsFile, track, hands, lanes, idempotencyKey, mode }, artifact };
  }

  async submit(req: SubmitRequest): Promise<{ job: Job; deduped: boolean }> {
    const { input, artifact } = this.validate(req);
    const existing = await this.d.store.byKey(input.idempotencyKey);
    if (existing) return { job: existing, deduped: true };

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
    this.d.log("job_accepted", { job: job.id, user: input.userId, track: input.track, hands: input.hands, mode: input.mode, reserved_micro: amount });
    const p = this.run(job.id, artifact).catch((e) => this.d.log("job_run_crashed", { job: job.id, error: String(e) }));
    this.running.set(job.id, p);
    void p.finally(() => this.running.delete(job.id));
    return { job, deduped: false };
  }

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
