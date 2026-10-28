// Pricing and cost metering.
//
//   PRICE  (what the player pays): credits per hand played. `PRICE_USD_PER_1K_HANDS`, integer micro-USD per job.
//          reserve = price of the hands requested (rounded up to whole deals), charged = price of hands actually played.
//   COGS   (what the job cost us, recorded for margin, never billed): sandbox seconds x shape x our Platinum rates,
//          max'ed with Platinum's own number read just before delete, plus an amortised per-hand share of the fixed
//          arena / bigmem host.
//
// Why we meter COGS ourselves instead of trusting Platinum's number: GET /v1/sandboxes/:id/usage 404s once the sandbox
// is deleted (apps/api/src/api/sandboxes.ts:5640-5643) and Platinum settles usage in 60 s cycles, so a short run may
// under-report right before delete.
import type { Config } from "./config.ts";
import { usdToMicro } from "./money.ts";

export type Pricing = Config["pricing"];

/** Deals are 6 hands (one per rotation); the arena rounds a request up to a multiple of 6. */
export const roundHands = (hands: number): number => Math.ceil(hands / 6) * 6;

export function pricePerHandMicro(p: Pricing): number {
  return (p.usdPerKHands * 1_000_000) / 1000;
}

/** What to hold before the job starts: the price of every hand we might be asked to play. */
export function reserveMicro(p: Pricing, handsRequested: number): number {
  return Math.max(usdToMicro(p.minChargeUsd), Math.ceil(roundHands(handsRequested) * pricePerHandMicro(p)));
}

/** What to charge once the job settled. `handsPlayed` comes from the arena (authoritative), never from the bot. */
export function chargeMicro(p: Pricing, handsPlayed: number): number {
  if (!(handsPlayed > 0)) return 0;
  return Math.max(usdToMicro(p.minChargeUsd), Math.round(handsPlayed * pricePerHandMicro(p)));
}

export interface CogsInput {
  cpu: number;
  ramMb: number;
  seconds: number;           // sandbox running time
  handsPlayed: number;
  platinumUsd?: number;      // from sandbox.usage(), if parsable
}

export function platinumUsdFromUsage(u: Record<string, unknown> | undefined): number | undefined {
  if (!u) return undefined;
  for (const k of ["cost_usd", "total_cost_usd", "cost", "total_usd"]) {
    const v = u[k];
    if (typeof v === "number" && Number.isFinite(v)) return v;
  }
  return undefined;
}

export function sandboxCogsUsd(p: Pricing, i: CogsInput): number {
  const local = i.seconds * (i.cpu * p.usdPerVcpuSecond + (i.ramMb / 1024) * p.usdPerGibSecond);
  return Math.max(local, i.platinumUsd ?? 0);
}

/** Our cost for the job, micro-USD. */
export function cogsMicro(p: Pricing, i: CogsInput): number {
  return usdToMicro(sandboxCogsUsd(p, i) + i.handsPlayed * p.usdPerHandArenaShare);
}
