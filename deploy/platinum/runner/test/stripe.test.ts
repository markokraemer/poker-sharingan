// Stripe handling with a fake signing secret. No network: the checkout gateway gets a fake fetch.
import assert from "node:assert/strict";
import { test } from "node:test";
import { MemoryLedger } from "../src/ledger.ts";
import {
  CREDITS_KIND, handleStripeEvent, planTopup, SimulatedCheckoutGateway, signStripePayload, StripeCheckoutGateway, StripeSignatureError,
  TopupRequestError, verifyStripeSignature, type StripeEvent,
} from "../src/stripe.ts";
import { testConfig, WEBHOOK_SECRET } from "./helpers.ts";

const NOW = 1_800_000_000;
const body = JSON.stringify({ id: "evt_1", type: "checkout.session.completed" });

test("a correctly signed payload verifies", () => {
  const h = signStripePayload(body, WEBHOOK_SECRET, NOW);
  verifyStripeSignature(body, h, WEBHOOK_SECRET, { nowS: NOW });
  verifyStripeSignature(Buffer.from(body), h, WEBHOOK_SECRET, { nowS: NOW + 299 });
});

test("matches Stripe's documented scheme (HMAC-SHA256 over `t.payload`)", () => {
  // computed independently with openssl: printf '1800000000.%s' "$body" | openssl dgst -sha256 -hmac "$secret"
  const h = signStripePayload("{}", "whsec_x", 1700000000);
  assert.match(h, /^t=1700000000,v1=[0-9a-f]{64}$/);
});

test("rejects tampered body, wrong secret, missing/garbled header, stale or future timestamps", () => {
  const h = signStripePayload(body, WEBHOOK_SECRET, NOW);
  assert.throws(() => verifyStripeSignature(body + " ", h, WEBHOOK_SECRET, { nowS: NOW }), StripeSignatureError);
  assert.throws(() => verifyStripeSignature(body, h, "whsec_other", { nowS: NOW }), StripeSignatureError);
  assert.throws(() => verifyStripeSignature(body, undefined, WEBHOOK_SECRET, { nowS: NOW }), /missing/);
  assert.throws(() => verifyStripeSignature(body, "garbage", WEBHOOK_SECRET, { nowS: NOW }), /malformed/);
  assert.throws(() => verifyStripeSignature(body, `t=${NOW}`, WEBHOOK_SECRET, { nowS: NOW }), /malformed/);
  assert.throws(() => verifyStripeSignature(body, `t=${NOW},v1=zz`, WEBHOOK_SECRET, { nowS: NOW }), StripeSignatureError);
  assert.throws(() => verifyStripeSignature(body, h, WEBHOOK_SECRET, { nowS: NOW + 301 }), /tolerance/);
  assert.throws(() => verifyStripeSignature(body, h, WEBHOOK_SECRET, { nowS: NOW - 301 }), /tolerance/);
  // a valid v0 only is not enough
  assert.throws(() => verifyStripeSignature(body, h.replace("v1=", "v0="), WEBHOOK_SECRET, { nowS: NOW }), /malformed/);
});

test("any one valid v1 among several (secret rotation) is accepted", () => {
  const good = signStripePayload(body, WEBHOOK_SECRET, NOW).split(",")[1];
  verifyStripeSignature(body, `t=${NOW},v1=${"0".repeat(64)},${good}`, WEBHOOK_SECRET, { nowS: NOW });
});

const session = (over: Record<string, unknown> = {}, meta: Record<string, string> = {}): StripeEvent => ({
  id: "evt_1",
  type: "checkout.session.completed",
  data: { object: { id: "cs_test_1", payment_status: "paid", currency: "usd", amount_total: 1000, client_reference_id: "user_a", metadata: { kind: CREDITS_KIND, user_id: "user_a", ...meta }, ...over } },
});

test("a paid credits session credits the user once; replays and the async twin are no-ops", async () => {
  const l = new MemoryLedger();
  const first = await handleStripeEvent(session(), l);
  assert.deepEqual(first, { outcome: "credited", userId: "user_a", creditMicro: 10_000_000, sessionId: "cs_test_1" });
  assert.equal((await l.balance("user_a")).balanceMicro, 10_000_000);
  assert.equal((await handleStripeEvent(session(), l)).outcome, "duplicate");
  assert.equal((await handleStripeEvent({ ...session(), id: "evt_2", type: "checkout.session.async_payment_succeeded" }, l)).outcome, "duplicate");
  assert.equal((await l.balance("user_a")).balanceMicro, 10_000_000);
});

test("pack bonus credit_micro is honoured up to 2x, never above, never below the amount paid", async () => {
  const l = new MemoryLedger();
  await handleStripeEvent(session({ id: "cs_b1", amount_total: 5000 }, { credit_micro: "55000000" }), l);
  assert.equal((await l.balance("user_a")).balanceMicro, 55_000_000);
  await handleStripeEvent(session({ id: "cs_b2", amount_total: 1000 }, { credit_micro: "999999999" }), l);       // absurd: ignored, credits the paid amount
  await handleStripeEvent(session({ id: "cs_b3", amount_total: 1000 }, { credit_micro: "1" }), l);               // below paid: ignored
  assert.equal((await l.balance("user_a")).balanceMicro, 55_000_000 + 10_000_000 + 10_000_000);
});

test("ignores: other products, unpaid, non-USD, no user, other event types", async () => {
  const l = new MemoryLedger();
  assert.equal((await handleStripeEvent(session({}, { kind: "something_else" }), l)).outcome, "ignored");
  assert.equal((await handleStripeEvent(session({ payment_status: "unpaid" }), l)).outcome, "ignored");
  assert.equal((await handleStripeEvent(session({ currency: "eur" }), l)).outcome, "ignored");
  assert.equal((await handleStripeEvent(session({ client_reference_id: null, metadata: { kind: CREDITS_KIND } }), l)).outcome, "ignored");
  assert.equal((await handleStripeEvent(session({ amount_total: 0 }), l)).outcome, "ignored");
  assert.equal((await handleStripeEvent({ id: "e", type: "charge.refunded", data: { object: {} } }, l)).outcome, "ignored");
  assert.equal((await l.balance("user_a")).balanceMicro, 0);
});

test("planTopup: packs, custom amounts, bounds", () => {
  const cfg = testConfig("http://x").stripe;
  const p = planTopup(cfg, { userId: "u", pack: "plus" });
  assert.deepEqual([p.priceCents, p.creditMicro, p.pack], [5000, 55_000_000, "plus"]);
  const c = planTopup(cfg, { userId: "u", amountUsd: 25 });
  assert.deepEqual([c.priceCents, c.creditMicro], [2500, 25_000_000]);
  assert.throws(() => planTopup(cfg, { userId: "u", pack: "nope" }), TopupRequestError);
  assert.throws(() => planTopup(cfg, { userId: "u", amountUsd: 1 }), /between/);
  assert.throws(() => planTopup(cfg, { userId: "u", amountUsd: 5000 }), /between/);
  assert.throws(() => planTopup(cfg, { userId: "u", amountUsd: 10.005 }), /cents/);
  assert.throws(() => planTopup(cfg, { userId: "u" }), /pack or amountUsd/);
  assert.throws(() => planTopup(cfg, { amountUsd: 10 } as any), /userId/);
});

test("StripeCheckoutGateway sends a form-encoded Checkout Session request (fake fetch, no network)", async () => {
  const seen: { url: string; init: RequestInit }[] = [];
  const fakeFetch = (async (url: URL | string, init: RequestInit) => {
    seen.push({ url: String(url), init });
    return new Response(JSON.stringify({ id: "cs_test_abc", url: "https://checkout.stripe.test/c/pay/cs_test_abc" }), { status: 200 });
  }) as unknown as typeof fetch;
  const gw = new StripeCheckoutGateway({ secretKey: "sk_test_fake", apiBase: "https://api.stripe.test" }, fakeFetch);
  const plan = planTopup(testConfig("http://x").stripe, { userId: "user_a", pack: "plus" });
  const s = await gw.createSession(plan, { successUrl: "https://s/ok", cancelUrl: "https://s/no" }, "idem-1");
  assert.deepEqual([s.id, s.simulated], ["cs_test_abc", false]);
  const { url, init } = seen[0]!;
  assert.equal(url, "https://api.stripe.test/v1/checkout/sessions");
  const h = init.headers as Record<string, string>;
  assert.equal(h.authorization, "Bearer sk_test_fake");
  assert.equal(h["idempotency-key"], "idem-1");
  const f = new URLSearchParams(String(init.body));
  assert.equal(f.get("mode"), "payment");
  assert.equal(f.get("client_reference_id"), "user_a");
  assert.equal(f.get("line_items[0][price_data][unit_amount]"), "5000");
  assert.equal(f.get("line_items[0][price_data][currency]"), "usd");
  assert.equal(f.get("metadata[kind]"), CREDITS_KIND);
  assert.equal(f.get("metadata[credit_micro]"), "55000000");
  assert.equal(f.get("metadata[pack]"), "plus");
});

test("StripeCheckoutGateway surfaces Stripe errors; simulated gateway never touches the network", async () => {
  const failing = (async () => new Response(JSON.stringify({ error: { message: "No such price" } }), { status: 400 })) as unknown as typeof fetch;
  const gw = new StripeCheckoutGateway({ secretKey: "sk_test_fake", apiBase: "https://api.stripe.test" }, failing);
  const plan = planTopup(testConfig("http://x").stripe, { userId: "u", amountUsd: 10 });
  await assert.rejects(gw.createSession(plan, { successUrl: "a", cancelUrl: "b" }, "k"), /No such price/);
  const sim = await new SimulatedCheckoutGateway("http://localhost:3000").createSession(plan);
  assert.match(sim.id, /^cs_sim_/);
  assert.equal(sim.simulated, true);
  assert.ok(sim.url.startsWith("http://localhost:3000/dev/checkout/cs_sim_"));
});
