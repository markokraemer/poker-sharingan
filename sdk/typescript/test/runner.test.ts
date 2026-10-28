import WebSocket from "ws";
import { afterEach, describe, expect, it } from "vitest";
import { ArenaClient, ApiException, Bot, CallingStation, LLMBot, RandomBot, Runner, TightAggressive, handleWebhook, sign, wsUrl, call, check, raiseTo, type Action, type BotLike, type YourTurn } from "../src/index.js";
import { MockArena } from "../src/mock.js";
import { DOC_YOUR_TURN } from "./fixtures.js";

let arena: MockArena | undefined;
afterEach(async () => { await arena?.close(); arena = undefined; });
const play = async (bot: BotLike, hands: number, o: Partial<ConstructorParameters<typeof Runner>[3]> = {}) => {
  const r = new Runner(bot, arena!.wsUrl, "any-token", { maxHands: hands, ...o });
  await r.run();
  return r;
};

describe("wsUrl", () => {
  it("normalizes", () => {
    expect(wsUrl("http://h:1", "tk")).toBe("ws://h:1/v1/play?token=tk");
    expect(wsUrl("https://h", "tk")).toBe("wss://h/v1/play?token=tk");
    expect(wsUrl("wss://h/v1/play?token=x", "tk")).toBe("wss://h/v1/play?token=x");
  });
});

describe("REST", () => {
  it("register / table / seat / errors", async () => {
    arena = await MockArena.start();
    const api = new ArenaClient(arena.httpUrl);
    const me = await api.registerBot("my-bot");
    expect(me.token && api.token === me.token).toBeTruthy();
    const t = await api.createTable({ small_blind: 50 });
    expect(t.config.big_blind).toBe(100);
    expect(t.seats).toHaveLength(6);
    const seat = await api.joinTable(t.table_id, 2, me.player_id);
    expect(seat.status).toBe("seated");
    await expect(api.joinTable(t.table_id, 2, me.player_id)).rejects.toMatchObject({ code: "seat_taken", status: 409 });
    expect((await api.getTable(t.table_id)).seats[2]!.name).toBe("my-bot");
    await expect(new ArenaClient(arena.httpUrl, "nope").getTable(t.table_id)).rejects.toBeInstanceOf(ApiException);
  });
});

describe("full sessions", () => {
  const makers: Array<[string, () => BotLike]> = [
    ["random", () => new RandomBot(1)], ["calling_station", () => new CallingStation()], ["tag", () => new TightAggressive()],
    ["llm", () => new LLMBot(() => "ACTION: call")],
  ];
  for (const [name, make] of makers)
    it(name, async () => {
      arena = await MockArena.start({ hands: 4 });
      const r = await play(make(), 4);
      expect(r.handsPlayed).toBe(4);
      expect(arena.handEnds).toHaveLength(4);
      expect(arena.errorsSent).toEqual([]);
      expect(r.netChips).toBe(arena.handEnds.reduce((a, h) => a + h.results[0].net, 0));
      for (const h of arena.handEnds) expect(h.results.reduce((a: number, x: any) => a + x.net, 0)).toBe(0);
    });

  it("hooks fire in order", async () => {
    const ev: any[] = [];
    const bot: BotLike = {
      act: (s) => (s.legal.call ? call() : check()),
      onHandStart: (h) => { ev.push(["start", h.hand_no]); },
      onStreet: (s) => { ev.push(["street", s.street, s.board.join(",")]); },
      onHandEnd: async (e) => { ev.push(["end", e.ended_by, e.hole_cards["0"]]); },
    };
    arena = await MockArena.start({ hands: 1 });
    await play(bot, 1);
    expect(ev[0][0]).toBe("start");
    expect(ev).toContainEqual(["street", "flop", "7c,8d,2s"]);
    expect(ev.at(-1)).toEqual(["end", "showdown", ["Ah", "Kd"]]);
  });

  it("reconnects and resumes the same decision", async () => {
    arena = await MockArena.start({ hands: 3, dropOnTurns: [1, 3] });
    const r = await play(new CallingStation(), 3);
    expect(r.handsPlayed).toBe(3);
    expect(arena.connections).toBeGreaterThanOrEqual(3);
    const hellos = arena.received.filter((m) => m.type === "hello");
    expect(hellos[0].data.resume).toBeUndefined();
    expect(hellos[1].data.resume[0].table_id).toBe("t1");
    expect(hellos[1].data.resume[0].last_table_seq).toBeGreaterThan(0);
    const ids = arena.acts.map((a) => a.decision_id);
    expect(ids).toHaveLength(6);
    expect(new Set(ids).size).toBe(6);
  });

  it("strict bot: error then retry on the same decision_id", async () => {
    let n = 0;
    const errs: string[] = [];
    const bot: BotLike = { strict: true, act: (s) => (++n === 1 ? raiseTo(5) : s.legal.call ? call() : check()), onError: (e) => { errs.push(e.code); } };
    arena = await MockArena.start({ hands: 1 });
    await play(bot, 1);
    expect(arena.errorsSent).toEqual(["bad_amount"]);
    expect(errs).toEqual(["bad_amount"]);
    expect(n).toBeGreaterThanOrEqual(3);
  });

  it("runner legalizes sloppy actions by default", async () => {
    const bot: BotLike = { act: (s) => (s.legal.raise ? raiseTo(5) : check()) };
    arena = await MockArena.start({ hands: 2 });
    await play(bot, 2);
    expect(arena.errorsSent).toEqual([]);
    expect(arena.acts[0].to).toBe(200);
  });

  it("slow or throwing act falls back to check/fold", async () => {
    let n = 0;
    const bot: BotLike = { act: async () => { if (++n === 1) await new Promise((r) => setTimeout(r, 5000)); throw new Error("boom"); } };
    arena = await MockArena.start({ hands: 1, decisionMs: 500 });
    await play(bot, 1, { safetyMarginMs: 100 });
    expect(arena.acts[0].type).toBe("fold");
  });

  it("table_seq gap triggers a resync hello", async () => {
    arena = await MockArena.start({ hands: 2, emitGapAt: 6 });
    await play(new CallingStation(), 2);
    const hellos = arena.received.filter((m) => m.type === "hello");
    expect(hellos.length).toBeGreaterThanOrEqual(2);
    expect(hellos[1].data.resume).toBeDefined();
  });

  it("works with the `ws` package injected", async () => {
    arena = await MockArena.start({ hands: 1 });
    const r = await play(new CallingStation(), 1, { WebSocket: WebSocket as any });
    expect(r.handsPlayed).toBe(1);
  });

  it("bad token: no welcome", async () => {
    arena = await MockArena.start();
    const r = new Runner(new CallingStation(), arena.wsUrl, "bad", { reconnect: false });
    await r.run();
    expect(r.welcome).toBeNull();
  });

  it("async LLM completion end to end", async () => {
    arena = await MockArena.start({ hands: 2 });
    await play(new LLMBot(async () => "Strong.\nACTION: raise 300"), 2);
    expect(arena.acts[0]).toMatchObject({ type: "raise", to: 300, hand_id: "h-1" });
  });
});

describe("webhook", () => {
  it("signs, verifies, answers", async () => {
    const body = JSON.stringify(DOC_YOUR_TURN);
    expect(await handleWebhook(new CallingStation(), body, sign("s3", body), "s3")).toEqual([200, { hand_id: "h-77", decision_id: "d-77-9", type: "check" }]);
    expect((await handleWebhook(new CallingStation(), body, "sha256=bad", "s3"))[0]).toBe(401);
    expect(await handleWebhook(new CallingStation(), '{"type":"action","data":{}}')).toEqual([200, {}]);
  });
});
