import { Bot } from "../bot.js";
import { parseActionText, renderState, safeAction } from "../helpers.js";
import type { Action, YourTurn } from "../types.js";

export const SYSTEM_PROMPT =
  "You are an expert no-limit hold'em player. Think briefly, then end your reply with exactly one final line " +
  "of the form `ACTION: fold`, `ACTION: check`, `ACTION: call`, `ACTION: raise <to>` or `ACTION: all-in`. " +
  "<to> is the TOTAL chips you will have in the pot this street after raising (not the increment) and must be " +
  "within the legal range shown.";

/** An LLM plays: the state is rendered as text, YOU supply `complete(prompt) => text` (any provider; no keys in the SDK). */
export class LLMBot extends Bot {
  name = "llm_bot";
  lastPrompt = "";
  lastReply = "";
  constructor(private complete: (prompt: string) => string | Promise<string>, private systemPrompt = SYSTEM_PROMPT) { super(); }

  buildPrompt(s: YourTurn): string { return `${this.systemPrompt}\n\n${renderState(s)}\n\nYour decision?`; }

  async act(s: YourTurn): Promise<Action> {
    this.lastPrompt = this.buildPrompt(s);
    this.lastReply = await this.complete(this.lastPrompt);
    const tail = this.lastReply.includes("ACTION:") ? this.lastReply.split("ACTION:").pop()! : this.lastReply;
    // the runner legalizes (clamps amounts, impossible raise -> call/check); unparsable -> check/fold
    return parseActionText(tail) ?? safeAction(s);
  }
}
