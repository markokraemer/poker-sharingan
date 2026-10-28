// HTTP skin over Service. Called only by our own web / billing backend (Bearer RUNNER_AUTH_TOKEN), except the Stripe
// webhook, which is authenticated by its signature.
//
//   GET  /healthz                         liveness
//   GET  /readyz                          200 when the database answers
//   POST /v1/webhooks/stripe              Stripe-Signature verified; checkout.session.completed -> credits
//   GET  /v1/credits/:userId              balance
//   GET  /v1/credits/:userId/entries      ledger entries (newest first)
//   POST /v1/credits/checkout             {userId, pack | amountUsd, successUrl?, cancelUrl?} -> {id, url}
//   GET  /v1/quote?hands=N                price of a run
//   POST /v1/jobs                         submit (credits reserved first; 402 when short)  -> 202 {id, status}
//   GET  /v1/jobs?userId=U                recent jobs
//   GET  /v1/jobs/:id                     status, progress, result, cost
//   POST /v1/jobs/:id/abort
import { createHash, timingSafeEqual } from "node:crypto";
import type { ArenaClient } from "./arena.ts";
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import type { Config } from "./config.ts";
import type { Logger } from "./log.ts";
import { microToUsd } from "./money.ts";
import { Service, ServiceError } from "./service.ts";
import type { Job } from "./store.ts";

export function jobView(j: Job) {
  return {
    id: j.id,
    status: j.status,
    mode: j.input.mode,
    user_id: j.input.userId,
    bot_name: j.input.botName,
    track: j.input.track,
    hands: j.input.hands,
    hands_done: j.handsDone ?? 0,
    hands_total: j.handsTotal ?? j.input.hands,
    run_id: j.runId ?? null,
    verified: j.verified ?? false,
    forfeit_hands: j.forfeitHands ?? null,
    arena_player_id: j.arenaPlayerId ?? null,
    mbb_per_game: j.mbbPerGame ?? null,
    ci95_mbb: j.ci95Mbb ?? null,
    running_mbb_per_game: j.runningMbb ?? null,
    error: j.error ?? null,
    error_class: j.errorClass ?? null,
    sandbox_id: j.sandboxId ?? null,
    sandbox_deleted: j.sandboxDeleted ?? null,
    sandbox_seconds: j.sandboxSeconds ?? null,
    reserved_micro: j.reservedMicro,
    charge_micro: j.chargeMicro ?? null,
    released_micro: j.releasedMicro ?? null,
    charge_usd: j.chargeMicro === undefined ? null : microToUsd(j.chargeMicro),
    settled: j.settled,
    cogs_micro: j.cogsMicro ?? null,
    platinum_usd: j.platinumUsd ?? null,
    bot_log_tail: j.botLogTail ?? null,
    created_at: new Date(j.createdAt).toISOString(),
    finished_at: j.finishedAt ? new Date(j.finishedAt).toISOString() : null,
  };
}

class HttpError extends Error {
  status: number;
  code: string;
  constructor(status: number, code: string, message: string) { super(message); this.status = status; this.code = code; }
}

const digest = (s: string) => createHash("sha256").update(s).digest();

async function readBody(req: IncomingMessage, limit: number): Promise<Buffer> {
  const chunks: Buffer[] = [];
  let n = 0;
  for await (const c of req) {
    n += (c as Buffer).length;
    if (n > limit) throw new HttpError(413, "payload_too_large", `body larger than ${limit} bytes`);
    chunks.push(c as Buffer);
  }
  return Buffer.concat(chunks);
}

export function createApp(svc: Service, cfg: Config, opts: { ready?: () => Promise<void>; log?: Logger; arena?: ArenaClient } = {}): Server {
  const jobBodyLimit = Math.ceil(cfg.limits.maxArtifactBytes * 1.4) + 64 * 1024;
  const want = digest(cfg.authToken);
  const bearerOf = (req: IncomingMessage) => (req.headers.authorization ?? "").replace(/^Bearer /i, "").trim();
  const authed = (req: IncomingMessage) => timingSafeEqual(digest(bearerOf(req)), want);
  // FREE mode: a public player authenticates with their arena bot token; validated against the arena (GET /v1/me), cached briefly.
  const tokenCache = new Map<string, { playerId: string | null; exp: number }>();
  const playerOf = async (req: IncomingMessage): Promise<string | undefined> => {
    if (cfg.billing.mode !== "free" || !opts.arena) return undefined;
    const tok = bearerOf(req);
    if (!tok || tok.length > 200) return undefined;
    const key = digest(tok).toString("hex");
    const hit = tokenCache.get(key);
    if (hit && hit.exp > Date.now()) return hit.playerId ?? undefined;
    const who = await opts.arena.whoami(tok).catch(() => undefined);
    const playerId = who && !who.admin ? who.playerId : null;
    if (tokenCache.size > 5000) tokenCache.clear();
    tokenCache.set(key, { playerId, exp: Date.now() + (playerId ? cfg.billing.authCacheS : 10) * 1000 });
    return playerId ?? undefined;
  };

  const send = (res: ServerResponse, code: number, body: unknown) => {
    const text = JSON.stringify(body);
    res.writeHead(code, { "content-type": "application/json", "content-length": Buffer.byteLength(text) });
    res.end(text);
  };
  const json = async (req: IncomingMessage, limit = 64 * 1024): Promise<any> => {
    const raw = await readBody(req, limit);
    try { return raw.length ? JSON.parse(raw.toString("utf8")) : {}; } catch { throw new HttpError(400, "bad_request", "body is not JSON"); }
  };

  return createServer(async (req, res) => {
    try {
      const url = new URL(req.url ?? "/", "http://x");
      const path = url.pathname;
      const method = req.method ?? "GET";

      if (path === "/healthz") return send(res, 200, { ok: true, backend: svc.d.platinum.kind, in_flight: svc.inFlight() });
      if (path === "/readyz") {
        try { await opts.ready?.(); return send(res, 200, { ok: true }); } catch (e) { return send(res, 503, { ok: false, error: String(e) }); }
      }

      if (path === "/v1/webhooks/stripe" && method === "POST" && !(cfg.billing.mode === "free" && !cfg.stripe.webhookSecret)) {
        const raw = await readBody(req, 1024 * 1024);
        const out = await svc.stripeWebhook(raw, req.headers["stripe-signature"] as string | undefined);
        return send(res, 200, { received: true, ...out });
      }

      // service token = trusted backend (any userId); in FREE mode an arena bot token = that one player, scoped to their own jobs
      let player: string | undefined;
      if (!authed(req)) {
        player = await playerOf(req);
        if (!player) return send(res, 401, { error: { code: "unauthorized", message: "bad or missing bearer token (use your arena bot token)" } });
      }
      const own = (userId: string) => { if (player && player !== userId) throw new ServiceError(403, "forbidden", "not your account"); };

      let m: RegExpMatchArray | null;
      if (method === "GET" && path === "/v1/quota") return send(res, 200, await svc.quota(player ?? (url.searchParams.get("userId") ?? "")));
      if (method === "GET" && (m = path.match(/^\/v1\/credits\/([^/]+)$/))) { own(decodeURIComponent(m[1]!)); return send(res, 200, await svc.credits(decodeURIComponent(m[1]!))); }
      if (method === "GET" && (m = path.match(/^\/v1\/credits\/([^/]+)\/entries$/))) {
        own(decodeURIComponent(m[1]!));
        return send(res, 200, { entries: await svc.entries(decodeURIComponent(m[1]!), Number(url.searchParams.get("limit") ?? 50)) });
      }
      if (method === "POST" && path === "/v1/credits/checkout") {
        if (player) throw new ServiceError(403, "forbidden", "service token only");
        const s = await svc.checkout(await json(req));
        return send(res, 201, { id: s.id, url: s.url, simulated: s.simulated, price_cents: s.plan.priceCents, credit_micro: s.plan.creditMicro });
      }
      if (method === "GET" && path === "/v1/quote") {
        const hands = Number(url.searchParams.get("hands") ?? cfg.limits.defaultHands);
        if (!Number.isInteger(hands) || hands < cfg.limits.minHands || hands > cfg.limits.maxHands) throw new ServiceError(400, "bad_request", "bad hands");
        return send(res, 200, svc.quote(hands));
      }
      if (method === "POST" && path === "/v1/jobs") {
        const { job, deduped } = await svc.submit(await json(req, jobBodyLimit), player);
        return send(res, deduped ? 200 : 202, { id: job.id, status: job.status, deduped, reserved_micro: job.reservedMicro });
      }
      if (method === "GET" && path === "/v1/jobs") {
        const userId = player ?? url.searchParams.get("userId");
        if (!userId) throw new ServiceError(400, "bad_request", "userId required");
        return send(res, 200, { jobs: (await svc.listJobs(userId, Number(url.searchParams.get("limit") ?? 20))).map(jobView) });
      }
      if (method === "GET" && (m = path.match(/^\/v1\/jobs\/([\w-]+)$/))) {
        const j = await svc.getJob(m[1]!);
        if (j && player && j.input.userId !== player) return send(res, 404, { error: { code: "not_found", message: "no such job" } });
        return j ? send(res, 200, jobView(j)) : send(res, 404, { error: { code: "not_found", message: "no such job" } });
      }
      if (method === "POST" && (m = path.match(/^\/v1\/jobs\/([\w-]+)\/abort$/))) {
        if (player) { const j = await svc.getJob(m[1]!); if (!j || j.input.userId !== player) return send(res, 404, { error: { code: "not_found", message: "no such job" } }); }
        const aborted = await svc.abort(m[1]!);
        return send(res, 202, { ok: true, aborted });
      }
      return send(res, 404, { error: { code: "not_found", message: `${method} ${path}` } });
    } catch (e) {
      if (e instanceof ServiceError) return send(res, e.status, { error: { code: e.code, message: e.message, ...(e.details ? { details: e.details } : {}) } });
      if (e instanceof HttpError) return send(res, e.status, { error: { code: e.code, message: e.message } });
      opts.log?.("request_failed", { error: String((e as Error)?.stack ?? e) });
      return send(res, 500, { error: { code: "internal", message: "internal error" } });
    }
  });
}
