// Cost metering. UNTESTED.
//
// Why we meter ourselves instead of trusting Platinum's number:
//  * GET /v1/sandboxes/:id/usage 404s once the sandbox is deleted (apps/api/src/api/sandboxes.ts:5640-5643),
//    and Platinum settles usage in 60 s cycles, so a short run may under-report right before delete.
//  * So: primary = wall-clock (running -> delete) x shape x our rates; secondary = Platinum's own number read just
//    before delete, stored for reconciliation. Settle on max(primary, platinum) so we never sell below cost.
import { config } from "./config.ts";

export interface CostInput {
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
    if (typeof v === "number" && isFinite(v)) return v;
  }
  return undefined;
}

export function rawSandboxUsd(i: CostInput): number {
  const local = i.seconds * (i.cpu * config.usdPerVcpuSecond + (i.ramMb / 1024) * config.usdPerGibSecond);
  return Math.max(local, i.platinumUsd ?? 0);
}

/** What the player is charged, in USD. The arena/bigmem host is a fixed cost, so it is recovered per hand, not per second. */
export function chargeUsd(i: CostInput): number {
  return (rawSandboxUsd(i) + i.handsPlayed * config.usdPerHandArenaShare) * config.markup;
}

/** Upper bound used to RESERVE credits before the job starts (deadline x shape + full hands). */
export function reserveUsd(hands: number): number {
  return chargeUsd({ cpu: config.cpu, ramMb: config.ramMb, seconds: config.runDeadlineS, handsPlayed: hands });
}
