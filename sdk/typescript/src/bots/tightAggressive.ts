import { Bot } from "../bot.js";
import { allInAction, bbMultipleTo, callOrCheck, canRaise, check, fold, madeHandCategory, potOdds, potRaiseTo, preflopStrength, raiseTo, safeAction, toCall, myStack } from "../helpers.js";
import type { Action, YourTurn } from "../types.js";

/** Simple rules: few hands preflop, raise strong ones, bet made hands postflop, fold air to pressure. */
export class TightAggressive extends Bot {
  name = "tight_aggressive";
  act(s: YourTurn): Action { return s.street === "preflop" ? this.preflop(s) : this.postflop(s); }

  private preflop(s: YourTurn): Action {
    const score = preflopStrength(s.hole_cards);
    const facingRaise = toCall(s) > s.big_blind;
    if (score >= 10) {
      if (!canRaise(s)) return callOrCheck(s);
      if (toCall(s) > 0.3 * myStack(s)) return allInAction(s);
      return raiseTo(bbMultipleTo(s, facingRaise ? 9 : 3));
    }
    if (score >= 7) {
      if (!facingRaise && canRaise(s)) return raiseTo(bbMultipleTo(s, 3));
      return toCall(s) <= 0.1 * myStack(s) ? callOrCheck(s) : safeAction(s);
    }
    if (score >= 5 && !facingRaise) return callOrCheck(s);
    return safeAction(s);
  }

  private postflop(s: YourTurn): Action {
    const strength = madeHandCategory(s.hole_cards, s.board);
    if (strength >= 2 && canRaise(s)) return raiseTo(potRaiseTo(s, 0.75));
    if (strength === 1) {
      if (toCall(s) === 0 && canRaise(s)) return raiseTo(potRaiseTo(s, 0.5));
      return potOdds(s) <= 0.33 ? callOrCheck(s) : safeAction(s);
    }
    return toCall(s) === 0 ? check() : fold();
  }
}
