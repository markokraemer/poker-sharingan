import { Bot } from "../bot.js";
import { bbMultipleTo, call, check, fold, legalTypes, raiseTo } from "../helpers.js";
import type { Action, YourTurn } from "../types.js";

/** mulberry32: tiny seedable PRNG */
function rng(seed: number) {
  let a = seed >>> 0;
  return () => { a = (a + 0x6d2b79f5) >>> 0; let t = a; t = Math.imul(t ^ (t >>> 15), t | 1); t ^= t + Math.imul(t ^ (t >>> 7), t | 61); return ((t ^ (t >>> 14)) >>> 0) / 4294967296; };
}

/** Picks uniformly among legal action types; raises a random 2-10 bb (clamped). */
export class RandomBot extends Bot {
  name = "random";
  private r: () => number;
  constructor(seed = Date.now()) { super(); this.r = rng(seed); }
  act(s: YourTurn): Action {
    const types = legalTypes(s);
    const t = types[Math.floor(this.r() * types.length)]!;
    if (t === "raise") return raiseTo(bbMultipleTo(s, 2 + this.r() * 8));
    return t === "fold" ? fold() : t === "check" ? check() : call();
  }
}
