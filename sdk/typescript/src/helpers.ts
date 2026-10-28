import type { Action, Card, YourTurn } from "./types.js";

export const fold = (): Action => ({ type: "fold" });
export const check = (): Action => ({ type: "check" });
export const call = (): Action => ({ type: "call" });
export const raiseTo = (to: number): Action => ({ type: "raise", to: Math.round(to) });
export const bet = raiseTo;

const RANKS = "23456789TJQKA";
export const rankValue = (c: Card): number => RANKS.indexOf(c[0]!) + 2;

export type LegalType = "fold" | "check" | "call" | "raise";
export function legalTypes(s: YourTurn): LegalType[] {
  const out: LegalType[] = [];
  if (s.legal.fold) out.push("fold");
  if (s.legal.check) out.push("check");
  if (s.legal.call) out.push("call");
  if (s.legal.raise) out.push("raise");
  return out;
}

export const me = (s: YourTurn) => s.players.find((p) => p.seat === s.seat);
export const myStack = (s: YourTurn): number => me(s)?.stack ?? 0;
export const stackBB = (s: YourTurn): number => (s.big_blind ? myStack(s) / s.big_blind : 0);
/** Chips needed to call (0 if checking is free). */
export const toCall = (s: YourTurn): number => s.legal.call?.added ?? 0;
/** Equity needed to break even on a call: to_call / (pot + to_call). */
export const potOdds = (s: YourTurn): number => { const t = toCall(s); return t > 0 ? t / (s.pot + t) : 0; };
export const minRaiseTo = (s: YourTurn): number | null => s.legal.raise?.min_to ?? null;
export const maxRaiseTo = (s: YourTurn): number | null => s.legal.raise?.max_to ?? null;
export const canRaise = (s: YourTurn): boolean => s.legal.raise !== null;
/** Highest street contribution on the table (what a call brings us to). */
export const currentBet = (s: YourTurn): number => s.legal.call?.to ?? me(s)?.street_contrib ?? 0;

export function clampRaiseTo(s: YourTurn, to: number): number {
  const r = s.legal.raise;
  if (!r) throw new Error("raising is not legal");
  return Math.max(r.min_to, Math.min(r.max_to, Math.round(to)));
}
/** 'to' amount for a bet/raise of `fraction` x pot (after calling), clamped to the legal range. */
export const potRaiseTo = (s: YourTurn, fraction = 1): number =>
  clampRaiseTo(s, currentBet(s) + Math.round((s.pot + toCall(s)) * fraction));
export const bbMultipleTo = (s: YourTurn, bbs: number): number => clampRaiseTo(s, bbs * s.big_blind);

/** Check if free, else fold: the server's own timeout policy. */
export const safeAction = (s: YourTurn): Action => (s.legal.check ? check() : fold());
export const callOrCheck = (s: YourTurn): Action => (s.legal.call ? call() : s.legal.check ? check() : fold());
export const allInAction = (s: YourTurn): Action =>
  s.legal.raise ? raiseTo(s.legal.raise.max_to) : s.legal.call ? call() : safeAction(s);

/** Coerce any action into a legal one (never throws): fold-when-free -> check, impossible raise -> call/check, amounts clamped. */
export function legalize(s: YourTurn, a: Action): Action {
  switch (a.type) {
    case "bet":
    case "raise":
    case "all_in":
      if (!s.legal.raise) return callOrCheck(s);
      return raiseTo(clampRaiseTo(s, a.type === "all_in" || a.to === undefined ? s.legal.raise.max_to : a.to));
    case "call": return callOrCheck(s);
    case "check": return s.legal.check ? check() : fold();
    case "fold": return s.legal.check ? check() : fold();
    default: return safeAction(s);
  }
}

/** True if `a` can be sent as-is without the server rejecting it. */
export function isLegal(s: YourTurn, a: Action): boolean {
  switch (a.type) {
    case "fold": return s.legal.fold;
    case "check": return s.legal.check;
    case "call": return s.legal.call !== null;
    case "bet":
    case "raise": { const r = s.legal.raise; return !!r && a.to !== undefined && a.to >= r.min_to && a.to <= r.max_to; }
    default: return false;
  }
}

/** Chen formula score (-1..20). >= 10 premium, >= 7 strong, >= 5 playable. */
export function preflopStrength(hole: readonly Card[]): number {
  if (hole.length !== 2) return 0;
  const [a, b] = [rankValue(hole[0]!), rankValue(hole[1]!)].sort((x, y) => y - x) as [number, number];
  let pts = ({ 14: 10, 13: 8, 12: 7, 11: 6 } as Record<number, number>)[a] ?? a / 2;
  if (a === b) return Math.max(5, pts * 2);
  if (hole[0]![1] === hole[1]![1]) pts += 2;
  const gap = a - b - 1;
  pts -= ({ 0: 0, 1: 1, 2: 2, 3: 4 } as Record<number, number>)[gap] ?? 5;
  if (gap <= 1 && a < 12) pts += 1;
  return Math.round(pts * 2) / 2;
}

/** Coarse postflop strength: 0 air, 1 pair using a hole card, 2 two pair, 3 trips/straight/flush or better. Not a hand evaluator. */
export function madeHandCategory(hole: readonly Card[], board: readonly Card[]): number {
  const cards = [...hole, ...board];
  if (cards.length < 5) return 0;
  const rc = new Map<string, number>(), sc = new Map<string, number>();
  for (const c of cards) { rc.set(c[0]!, (rc.get(c[0]!) ?? 0) + 1); sc.set(c[1]!, (sc.get(c[1]!) ?? 0) + 1); }
  const counts = [...rc.values()].sort((x, y) => y - x);
  let vals = [...new Set(cards.map(rankValue))].sort((x, y) => x - y);
  if (vals.includes(14)) vals = [1, ...vals];
  const straight = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10].some((v) => [0, 1, 2, 3, 4].every((i) => vals.includes(v + i)));
  const flush = Math.max(...sc.values()) >= 5;
  if (counts[0]! >= 3 || straight || flush) return 3;
  if (counts[0] === 2 && counts[1] === 2) return 2;
  if (counts[0] === 2) {
    const paired = [...rc.entries()].filter(([, n]) => n === 2).map(([r]) => r);
    return paired.some((r) => r === hole[0]![0] || r === hole[1]![0]) ? 1 : 0;
  }
  return 0;
}

export function positionName(s: YourTurn): string {
  const seats = s.players.map((p) => p.seat);
  const order = [...seats].sort((a, b) => ((a - s.button_seat + 6) % 6) - ((b - s.button_seat + 6) % 6));
  const idx = order.indexOf(s.seat);
  if (idx < 0) return "?";
  if (order.length === 2) return idx === 0 ? "BTN/SB" : "BB";
  const fixed = ["BTN", "SB", "BB"];
  if (idx < 3) return fixed[idx]!;
  const rest = order.length - 3, pos = idx - 3;
  return pos === rest - 1 ? "CO" : pos === 0 ? "UTG" : "MP";
}

/** Plain-text rendering of the decision for LLM prompts or logs. */
export function renderState(s: YourTurn): string {
  const L: string[] = [];
  L.push(`Texas Hold'em no-limit, 6-max. Blinds ${s.small_blind}/${s.big_blind}. Street: ${s.street}.`);
  L.push(`You are seat ${s.seat} (${positionName(s)}), stack ${myStack(s)} (${stackBB(s).toFixed(1)} bb).`);
  L.push(`Your hole cards: ${s.hole_cards.join(" ")}`);
  L.push(`Board: ${s.board.length ? s.board.join(" ") : "(none)"}`);
  L.push(`Pot: ${s.pot}. To call: ${toCall(s)}. Pot odds: ${(potOdds(s) * 100).toFixed(1)}%.`);
  L.push("Players:");
  for (const p of s.players)
    L.push(`  seat ${p.seat} ${p.name}${p.seat === s.seat ? " (YOU)" : ""}: stack ${p.stack}, in pot this street ${p.street_contrib}, this hand ${p.hand_contrib}, ${p.status}`);
  L.push("Action so far:");
  for (const a of s.history) {
    const d = a.type === "fold" || a.type === "check" ? a.type : `${a.type} to ${a.to}${a.all_in ? " (all-in)" : ""}`;
    L.push(`  [${a.street}] seat ${a.seat}: ${d}`);
  }
  const o: string[] = [];
  if (s.legal.fold) o.push("fold");
  if (s.legal.check) o.push("check");
  if (s.legal.call) o.push(`call (adds ${s.legal.call.added}, to ${s.legal.call.to}${s.legal.call.all_in ? ", all-in" : ""})`);
  if (s.legal.raise) o.push(`raise (to an amount between ${s.legal.raise.min_to} and ${s.legal.raise.max_to}; amounts are TOTAL street contribution, not increments)`);
  L.push("Legal actions: " + o.join("; "));
  return L.join("\n");
}

/** Parse free-form LLM output ('raise 450', 'RAISE TO 450', 'call', 'all-in'). Uses the LAST recognised action. Null if none. */
export function parseActionText(text: string): Action | null {
  const re = /\b(fold|check|call|all[- ]?in|raise|bet)\b(?:\s*(?:to|=|:)?\s*\$?(\d+))?/g;
  let last: Action | null = null;
  for (const m of text.toLowerCase().matchAll(re)) {
    const w = m[1]!;
    last = w.startsWith("all") ? { type: "all_in" } : w === "raise" || w === "bet" ? (m[2] ? { type: "raise", to: Number(m[2]) } : { type: "raise" }) : ({ type: w } as Action);
  }
  return last;
}
