import { Bot } from "../bot.js";
import { callOrCheck } from "../helpers.js";
import type { Action, YourTurn } from "../types.js";

/** Never folds to a bet, never raises: checks when free, otherwise calls. */
export class CallingStation extends Bot {
  name = "calling_station";
  act(s: YourTurn): Action { return callOrCheck(s); }
}
