// Stripe, without the Stripe SDK and without any network call in tests:
//   * verifyStripeSignature: the `Stripe-Signature` header scheme (t=<unix>,v1=<hmac-sha256 hex>[,v1=...]) over `${t}.${rawBody}`
//   * handleStripeEvent:     checkout.session.completed / async_payment_succeeded -> ledger top-up (idempotent on the session id)
//   * CheckoutGateway:       StripeCheckoutGateway (REST, form-encoded, injectable fetch) or SimulatedCheckoutGateway (no network)
// https://docs.stripe.com/webhooks#verify-manually
import { createHmac, randomBytes, timingSafeEqual } from "node:crypto";
import type { Config, CreditPack } from "./config.ts";
import type { Ledger } from "./ledger.ts";
import { centsToMicro, usdToMicro } from "./money.ts";

export class StripeSignatureError extends Error {}
export class TopupRequestError extends Error {}

/** Marker our checkout sessions carry, so the shared webhook endpoint ignores every other Stripe product. */
export const CREDITS_KIND = "sharingan_credits";

// ---------------------------------------------------------------------------------------------- signatures

export function signStripePayload(rawBody: string | Buffer, secret: string, timestampS: number): string {
  const sig = createHmac("sha256", secret).update(`${timestampS}.`).update(rawBody).digest("hex");
  return `t=${timestampS},v1=${sig}`;
}

export function verifyStripeSignature(
  rawBody: string | Buffer,
  header: string | undefined,
  secret: string,
  opts: { toleranceS?: number; nowS?: number } = {},
): void {
  if (!header) throw new StripeSignatureError("missing Stripe-Signature header");
  let t: number | undefined;
  const v1: string[] = [];
  for (const part of header.split(",")) {
    const eq = part.indexOf("=");
    if (eq < 0) continue;
    const k = part.slice(0, eq).trim();
    const v = part.slice(eq + 1).trim();
    if (k === "t") t = Number(v);
    else if (k === "v1") v1.push(v);          // v0 (test-mode legacy) is deliberately ignored
  }
  if (t === undefined || !Number.isFinite(t) || v1.length === 0) throw new StripeSignatureError("malformed Stripe-Signature header");
  const expected = createHmac("sha256", secret).update(`${t}.`).update(rawBody).digest();
  const ok = v1.some((cand) => {
    if (!/^[0-9a-f]{64}$/i.test(cand)) return false;
    return timingSafeEqual(Buffer.from(cand, "hex"), expected);
  });
  if (!ok) throw new StripeSignatureError("signature mismatch");
  const now = opts.nowS ?? Math.floor(Date.now() / 1000);
  const tol = opts.toleranceS ?? 300;
  if (Math.abs(now - t) > tol) throw new StripeSignatureError(`timestamp outside the ${tol}s tolerance`);
}

// ---------------------------------------------------------------------------------------------- events

export interface StripeEvent {
  id: string;
  type: string;
  data: { object: Record<string, any> };
}

export type EventOutcome =
  | { outcome: "credited"; userId: string; creditMicro: number; sessionId: string }
  | { outcome: "duplicate"; userId: string; sessionId: string }
  | { outcome: "ignored"; reason: string };

export async function handleStripeEvent(event: StripeEvent, ledger: Ledger): Promise<EventOutcome> {
  if (event.type !== "checkout.session.completed" && event.type !== "checkout.session.async_payment_succeeded") {
    return { outcome: "ignored", reason: `event type ${event.type}` };
  }
  const s = event.data?.object ?? {};
  const meta: Record<string, string> = s.metadata ?? {};
  if (meta.kind !== CREDITS_KIND) return { outcome: "ignored", reason: "not a credits checkout" };
  if (s.payment_status !== "paid") return { outcome: "ignored", reason: `payment_status ${s.payment_status} (credited on async_payment_succeeded)` };
  if (String(s.currency ?? "").toLowerCase() !== "usd") return { outcome: "ignored", reason: `currency ${s.currency}` };
  const userId: string | undefined = s.client_reference_id || meta.user_id;
  if (!userId) return { outcome: "ignored", reason: "no user id on session" };
  const paidCents = Number(s.amount_total);
  if (!Number.isInteger(paidCents) || paidCents <= 0) return { outcome: "ignored", reason: "bad amount_total" };

  // Credits normally equal the amount paid; a pack may carry a bonus (metadata.credit_micro, set by us), capped at 2x.
  let creditMicro = centsToMicro(paidCents);
  if (meta.credit_micro !== undefined) {
    const m = Number(meta.credit_micro);
    if (Number.isSafeInteger(m) && m >= creditMicro && m <= creditMicro * 2) creditMicro = m;
  }
  const sessionId: string = s.id ?? event.id;
  const r = await ledger.topup(userId, creditMicro, `session:${sessionId}`, {
    source: "stripe", event: event.id, session: sessionId, paid_cents: paidCents, pack: meta.pack,
  });
  return r.applied ? { outcome: "credited", userId, creditMicro, sessionId } : { outcome: "duplicate", userId, sessionId };
}

// ---------------------------------------------------------------------------------------------- checkout

export interface TopupRequest { userId: string; pack?: string; amountUsd?: number; successUrl?: string; cancelUrl?: string }
export interface TopupPlan { userId: string; priceCents: number; creditMicro: number; label: string; pack?: string }
export interface CheckoutSession { id: string; url: string; simulated: boolean; plan: TopupPlan }

export function planTopup(cfg: Config["stripe"], req: TopupRequest): TopupPlan {
  if (!req.userId) throw new TopupRequestError("userId required");
  if (req.pack) {
    const pack: CreditPack | undefined = cfg.packs.find((p) => p.id === req.pack);
    if (!pack) throw new TopupRequestError(`unknown pack ${req.pack}; available: ${cfg.packs.map((p) => p.id).join(", ")}`);
    return { userId: req.userId, priceCents: Math.round(pack.priceUsd * 100), creditMicro: usdToMicro(pack.creditUsd), label: `Sharingan credits (${pack.label})`, pack: pack.id };
  }
  const usd = Number(req.amountUsd);
  if (!Number.isFinite(usd)) throw new TopupRequestError("pack or amountUsd required");
  if (usd < cfg.minTopupUsd || usd > cfg.maxTopupUsd) throw new TopupRequestError(`amountUsd must be between ${cfg.minTopupUsd} and ${cfg.maxTopupUsd}`);
  const cents = Math.round(usd * 100);
  if (Math.abs(cents - usd * 100) > 1e-6) throw new TopupRequestError("amountUsd must be a whole number of cents");
  return { userId: req.userId, priceCents: cents, creditMicro: centsToMicro(cents), label: `Sharingan credits ($${usd.toFixed(2)})` };
}

export interface CheckoutGateway {
  createSession(plan: TopupPlan, urls: { successUrl: string; cancelUrl: string }, idempotencyKey: string): Promise<CheckoutSession>;
}

/** Stripe Checkout over the REST API (test or live keys). The only code in the service that can reach api.stripe.com. */
export class StripeCheckoutGateway implements CheckoutGateway {
  private readonly cfg: Pick<Config["stripe"], "secretKey" | "apiBase">;
  private readonly doFetch: typeof fetch;
  constructor(cfg: Pick<Config["stripe"], "secretKey" | "apiBase">, doFetch: typeof fetch = fetch) {
    if (!cfg.secretKey) throw new Error("STRIPE_SECRET_KEY required");
    this.cfg = cfg;
    this.doFetch = doFetch;
  }

  async createSession(plan: TopupPlan, urls: { successUrl: string; cancelUrl: string }, idempotencyKey: string): Promise<CheckoutSession> {
    const form = new URLSearchParams();
    form.set("mode", "payment");
    form.set("success_url", urls.successUrl);
    form.set("cancel_url", urls.cancelUrl);
    form.set("client_reference_id", plan.userId);
    form.set("line_items[0][quantity]", "1");
    form.set("line_items[0][price_data][currency]", "usd");
    form.set("line_items[0][price_data][unit_amount]", String(plan.priceCents));
    form.set("line_items[0][price_data][product_data][name]", plan.label);
    form.set("metadata[kind]", CREDITS_KIND);
    form.set("metadata[user_id]", plan.userId);
    form.set("metadata[credit_micro]", String(plan.creditMicro));
    if (plan.pack) form.set("metadata[pack]", plan.pack);
    const res = await this.doFetch(new URL("/v1/checkout/sessions", this.cfg.apiBase), {
      method: "POST",
      headers: {
        authorization: `Bearer ${this.cfg.secretKey}`,
        "content-type": "application/x-www-form-urlencoded",
        "idempotency-key": idempotencyKey,
      },
      body: form.toString(),
      signal: AbortSignal.timeout(20_000),
    });
    const j = (await res.json().catch(() => ({}))) as { id?: string; url?: string; error?: { message?: string } };
    if (!res.ok || !j.id || !j.url) throw new Error(`stripe checkout failed (${res.status}): ${j.error?.message ?? "no session"}`);
    return { id: j.id, url: j.url, simulated: false, plan };
  }
}

/** No Stripe account: hands back a local URL. Complete it by POSTing a signed `checkout.session.completed` webhook. */
export class SimulatedCheckoutGateway implements CheckoutGateway {
  private readonly publicBaseUrl: string;
  constructor(publicBaseUrl: string) { this.publicBaseUrl = publicBaseUrl; }
  async createSession(plan: TopupPlan): Promise<CheckoutSession> {
    const id = `cs_sim_${randomBytes(9).toString("hex")}`;
    return { id, url: new URL(`/dev/checkout/${id}`, this.publicBaseUrl).toString(), simulated: true, plan };
  }
}

export function makeGateway(cfg: Config["stripe"], doFetch: typeof fetch = fetch): CheckoutGateway {
  return cfg.secretKey ? new StripeCheckoutGateway(cfg, doFetch) : new SimulatedCheckoutGateway(cfg.publicBaseUrl);
}
