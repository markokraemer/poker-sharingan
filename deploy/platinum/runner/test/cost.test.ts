import assert from "node:assert/strict";
import { test } from "node:test";
import { loadConfig } from "../src/config.ts";
import { chargeMicro, cogsMicro, platinumUsdFromUsage, reserveMicro, roundHands, sandboxCogsUsd } from "../src/cost.ts";
import { testConfig } from "./helpers.ts";

const cfg = testConfig("http://x");          // $1 per 1,000 hands

test("deals are 6 hands: requests round up", () => {
  assert.equal(roundHands(600), 600);
  assert.equal(roundHands(601), 606);
  assert.equal(roundHands(2000), 2004);
});

test("reserve = price of the rounded request, charge = price of hands played", () => {
  assert.equal(reserveMicro(cfg.pricing, 20_000), 20_004_000);       // 20,004 hands x 1000 micro
  assert.equal(reserveMicro(cfg.pricing, 2000), 2_004_000);          // 2004 hands x 1000 micro
  assert.equal(chargeMicro(cfg.pricing, 2004), 2_004_000);
  assert.equal(chargeMicro(cfg.pricing, 1000), 1_000_000);
  assert.equal(chargeMicro(cfg.pricing, 0), 0);
  assert.ok(chargeMicro(cfg.pricing, 1200) <= reserveMicro(cfg.pricing, 2000));
});

test("price is configurable via env", () => {
  const c = loadConfig({
    RUNNER_AUTH_TOKEN: "a", ARENA_URL: "http://x", ARENA_RUNNER_TOKEN: "r", STRIPE_WEBHOOK_SECRET: "s", RUNNER_STORAGE: "memory", SANDBOX_BACKEND: "local",
    PRICE_USD_PER_1K_HANDS: "0.05", MIN_JOB_CHARGE_USD: "0.01",
  });
  assert.equal(chargeMicro(c.pricing, 20_000), 1_000_000);           // $1 per 20k hands
  assert.equal(chargeMicro(c.pricing, 6), 10_000);                   // minimum charge applies once a hand was played
  assert.equal(chargeMicro(c.pricing, 0), 0);
  assert.equal(reserveMicro(c.pricing, 600), 30_000);
});

test("COGS: wall clock x shape, Platinum's own number wins when higher, plus per-hand arena share", () => {
  const p = cfg.pricing;
  const local = sandboxCogsUsd(p, { cpu: 2, ramMb: 2048, seconds: 100, handsPlayed: 0 });
  assert.ok(Math.abs(local - 100 * (2 * p.usdPerVcpuSecond + 2 * p.usdPerGibSecond)) < 1e-12);
  assert.equal(sandboxCogsUsd(p, { cpu: 2, ramMb: 2048, seconds: 100, handsPlayed: 0, platinumUsd: 5 }), 5);
  const withHands = cogsMicro(p, { cpu: 1, ramMb: 1024, seconds: 0, handsPlayed: 1_000_000 });
  assert.equal(withHands, Math.round(1_000_000 * p.usdPerHandArenaShare * 1e6));
});

test("platinumUsdFromUsage reads the first numeric cost field", () => {
  assert.equal(platinumUsdFromUsage({ cost_usd: 0.25 }), 0.25);
  assert.equal(platinumUsdFromUsage({ cost: "x", total_usd: 1 }), 1);
  assert.equal(platinumUsdFromUsage({}), undefined);
  assert.equal(platinumUsdFromUsage(undefined), undefined);
});

test("config refuses missing secrets", () => {
  assert.throws(() => loadConfig({ ARENA_URL: "http://x", RUNNER_STORAGE: "memory", SANDBOX_BACKEND: "local" }), /RUNNER_AUTH_TOKEN/);
  assert.throws(() => loadConfig({ RUNNER_AUTH_TOKEN: "a", ARENA_URL: "http://x", ARENA_RUNNER_TOKEN: "r", STRIPE_WEBHOOK_SECRET: "s", RUNNER_STORAGE: "memory" }), /PT_TOKEN/);
  assert.throws(() => loadConfig({ RUNNER_AUTH_TOKEN: "a", ARENA_URL: "http://x", ARENA_RUNNER_TOKEN: "r", STRIPE_WEBHOOK_SECRET: "s", SANDBOX_BACKEND: "local" }), /RUNNER_DATABASE_URL/);
});
