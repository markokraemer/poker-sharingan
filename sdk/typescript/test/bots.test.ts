import { describe, expect, it } from "vitest";
import { CallingStation, isLegal, LLMBot, RandomBot, TightAggressive, type YourTurn, check } from "../src/index.js";
import { docState } from "./fixtures.js";

const CARDS = [..."23456789TJQKA"].flatMap((r) => [..."cdhs"].map((s) => r + s));
let seed = 7;
const rnd = () => { seed = (seed * 1103515245 + 12345) & 0x7fffffff; return seed / 0x7fffffff; };
const pick = <T,>(a: T[]) => a[Math.floor(rnd() * a.length)]!;

function randomState(): YourTurn {
  const s = docState();
  const street = pick(["preflop", "flop", "turn", "river"] as const);
  const cards = [...CARDS].sort(() => rnd() - 0.5).slice(0, 7);
  s.hole_cards = [cards[0]!, cards[1]!]; s.street = street;
  s.board = cards.slice(2, 2 + { preflop: 0, flop: 3, turn: 4, river: 5 }[street]);
  const stack = pick([300, 2000, 9750]);
  s.players[0]!.stack = stack;
  if (rnd() < 0.6) {
    const amt = Math.min(pick([100, 250, 600, stack]), stack);
    s.legal = { fold: true, check: false, call: { to: amt, added: amt, all_in: amt === stack }, raise: amt >= stack || rnd() < 0.2 ? null : { min_to: Math.min(2 * amt, stack), max_to: stack } };
  } else s.legal = { fold: true, check: true, call: null, raise: { min_to: Math.min(100, stack), max_to: stack } };
  return s;
}

describe("bots", () => {
  it("always return legal actions", async () => {
    const bots = [new RandomBot(3), new CallingStation(), new TightAggressive()];
    for (let i = 0; i < 400; i++) {
      const s = randomState();
      for (const b of bots) expect(isLegal(s, await b.act(s)), `${b.name} ${JSON.stringify(s.legal)}`).toBe(true);
    }
  });
  it("basics", async () => {
    const s = docState();
    expect(new CallingStation().act(s)).toEqual(check());
    s.street = "preflop"; s.board = []; s.hole_cards = ["Ah", "Ad"];
    expect((await new TightAggressive().act(s)).type).toBe("raise");
    s.hole_cards = ["7h", "2d"];
    expect(["check", "fold"]).toContain((await new TightAggressive().act(s)).type);
  });
  it("llm bot builds prompt and parses reply", async () => {
    const seen: string[] = [];
    const bot = new LLMBot(async (p) => { seen.push(p); return "pot is small.\nACTION: raise 300"; });
    expect(await bot.act(docState())).toEqual({ type: "raise", to: 300 });
    expect(seen[0]).toContain("Your hole cards");
    expect(seen[0]).toContain("Legal actions");
  });
});
