import type { BotLike } from "./bot.js";
import { legalize, safeAction } from "./helpers.js";
import { wsUrl } from "./client.js";
import type { Action, ActionEvent, ApiError, Envelope, HandEnd, HandStart, StreetEvent, Table, YourTurn } from "./types.js";

export const CLIENT_NAME = "sharingan-ts/0.1.0";

/** Minimal WebSocket surface shared by browsers, Node >= 22 and the `ws` package. */
export interface WSLike {
  send(data: string): void;
  close(code?: number, reason?: string): void;
  onopen: ((ev: any) => void) | null;
  onmessage: ((ev: { data: any }) => void) | null;
  onclose: ((ev: any) => void) | null;
  onerror: ((ev: any) => void) | null;
}
export type WSCtor = new (url: string) => WSLike;

export interface RunnerOptions {
  /** Reconnect with `hello{resume}` after a drop (default true). */
  reconnect?: boolean;
  maxBackoffMs?: number;
  pingIntervalMs?: number;
  /** Reserve this much of the decision window for network latency (default 150). */
  safetyMarginMs?: number;
  /** Stop after this many hands. */
  maxHands?: number;
  /** Inject a WebSocket implementation (default: global WebSocket, else the `ws` package). */
  WebSocket?: WSCtor;
  onMessage?: (env: Envelope) => void;
  log?: (msg: string) => void;
}

async function resolveWS(opt?: WSCtor): Promise<WSCtor> {
  if (opt) return opt;
  const g = (globalThis as any).WebSocket;
  if (g) return g;
  try {
    const mod: any = await import("ws");
    return (mod.WebSocket ?? mod.default) as WSCtor;
  } catch {
    throw new Error("No WebSocket available: use Node >= 22, a browser, or `npm i ws`");
  }
}

/** Connects a bot to /v1/play: hello/resume, keep-alive, dedupe, gap resync, reconnect with backoff, deadline-aware `act`. */
export class Runner {
  readonly url: string;
  welcome: Record<string, any> | null = null;
  playerId: string | null = null;
  tables = new Map<string, Table>();
  lastTableSeq = new Map<string, number>();
  handsPlayed = 0;
  netChips = 0;
  connects = 0;
  errors: ApiError[] = [];

  private handSeq = new Map<string, number>();
  private answered = new Set<string>();
  private inflight = new Set<Promise<void>>();
  private mySeat = new Map<string, number>();
  private ws: WSLike | null = null;
  private stopped = false;
  private resyncing = false;
  private chain: Promise<void> = Promise.resolve();
  private opts: Required<Pick<RunnerOptions, "reconnect" | "maxBackoffMs" | "pingIntervalMs" | "safetyMarginMs">> & RunnerOptions;

  constructor(private bot: BotLike, url: string, token: string, opts: RunnerOptions = {}) {
    this.url = wsUrl(url, token);
    this.opts = { reconnect: true, maxBackoffMs: 10_000, pingIntervalMs: 15_000, safetyMarginMs: 150, ...opts };
  }

  stop(): void { this.stopped = true; this.ws?.close(); }

  async run(): Promise<void> {
    const WS = await resolveWS(this.opts.WebSocket);
    let backoff = 200;
    while (!this.stopped) {
      const opened = await this.connectOnce(WS);
      if (opened) backoff = 200;
      if (this.stopped || !this.opts.reconnect) break;
      await new Promise((r) => setTimeout(r, backoff));
      backoff = Math.min(this.opts.maxBackoffMs, backoff * 2);
    }
  }

  send(type: string, data: object, tableId?: string): void {
    const env: Envelope = { v: 1, type, t_ms: Date.now(), data, ...(tableId ? { table_id: tableId } : {}) };
    this.ws?.send(JSON.stringify(env));
  }
  leave(tableId: string): void { this.send("leave", { table_id: tableId }, tableId); }

  private connectOnce(WS: WSCtor): Promise<boolean> {
    return new Promise((resolve) => {
      let opened = false;
      let pinger: ReturnType<typeof setInterval> | undefined;
      const ws = new WS(this.url);
      this.ws = ws;
      ws.onopen = () => {
        opened = true;
        this.connects++;
        this.answered.clear();
        this.resyncing = false;
        this.hello(true);
        pinger = setInterval(() => { try { this.send("ping", {}); } catch { /* closed */ } }, this.opts.pingIntervalMs);
      };
      ws.onmessage = (ev) => {
        let env: Envelope;
        try { env = JSON.parse(typeof ev.data === "string" ? ev.data : String(ev.data)); } catch { return; }
        this.chain = this.chain.then(() => this.dispatch(env)).catch((e) => this.opts.log?.(`dispatch error: ${e}`));
      };
      let done = false;
      const finish = () => {
        if (done) return;
        done = true;
        if (pinger) clearInterval(pinger);
        if (this.ws === ws) this.ws = null;
        resolve(opened);
      };
      ws.onclose = finish;
      ws.onerror = () => { try { ws.close(); } catch { /* ignore */ } finish(); };
    });
  }

  private hello(resume: boolean): void {
    const data: Record<string, unknown> = { client: CLIENT_NAME };
    if (resume && this.lastTableSeq.size)
      data.resume = [...this.lastTableSeq].map(([table_id, last_table_seq]) => ({ table_id, last_table_seq }));
    this.send("hello", data);
  }

  /** Returns true if this message is a duplicate that must be dropped; may trigger a resync on gaps. */
  private isDuplicateOrGap(env: Envelope): boolean {
    const tid = env.table_id, ts = env.table_seq;
    if (tid && typeof ts === "number") {
      const last = this.lastTableSeq.get(tid);
      if (last !== undefined && ts <= last && env.type !== "your_turn") return true;
      if (last !== undefined && ts > last + 1 && !this.resyncing) { this.resyncing = true; this.hello(true); }
      this.lastTableSeq.set(tid, Math.max(ts, last ?? 0));
    }
    if (env.hand_id && typeof env.seq === "number" && env.type !== "your_turn") {
      const last = this.handSeq.get(env.hand_id);
      if (last !== undefined && env.seq > last + 1 && !this.resyncing) { this.resyncing = true; this.hello(true); }
      this.handSeq.set(env.hand_id, Math.max(env.seq, last ?? 0));
      if (this.handSeq.size > 64) this.handSeq.delete(this.handSeq.keys().next().value!);
    }
    return false;
  }

  private async hook<K extends keyof BotLike>(name: K, ...args: any[]): Promise<void> {
    const fn = this.bot[name] as ((...a: any[]) => any) | undefined;
    if (!fn) return;
    try { await fn.apply(this.bot, args); } catch (e) { this.opts.log?.(`bot.${String(name)} threw: ${e}`); }
  }

  private async dispatch(env: Envelope): Promise<void> {
    this.opts.onMessage?.(env);
    const data = env.data ?? {};
    const tid: string = env.table_id ?? data.table_id ?? "";
    switch (env.type) {
      case "pong": return;
      case "table_snapshot": {
        const t = data as Table;
        const id = t.table_id || tid;
        this.tables.set(id, t);
        this.lastTableSeq.set(id, t.table_seq ?? 0);
        this.handSeq.clear();
        this.resyncing = false;
        for (const s of t.seats ?? []) if (s.player_id && s.player_id === this.playerId) this.mySeat.set(id, s.seat);
        return;
      }
    }
    if (this.isDuplicateOrGap(env)) return;
    switch (env.type) {
      case "welcome": this.welcome = data; this.playerId = data.player_id ?? null; await this.hook("onConnect", data); break;
      case "hand_start": {
        const h = data as HandStart;
        for (const p of h.players ?? []) if (p.player_id && p.player_id === this.playerId) this.mySeat.set(tid, p.seat);
        await this.hook("onHandStart", h);
        break;
      }
      case "action": await this.hook("onAction", data as ActionEvent); break;
      case "street": await this.hook("onStreet", data as StreetEvent); break;
      case "hand_end": {
        const end = data as HandEnd;
        this.handsPlayed++;
        let seat = this.mySeat.get(tid);
        if (seat === undefined) seat = end.results?.find((r) => r.player_id === this.playerId)?.seat;
        if (seat !== undefined) this.netChips += end.results?.find((r) => r.seat === seat)?.net ?? 0;
        await this.hook("onHandEnd", end);
        if (this.opts.maxHands !== undefined && this.handsPlayed >= this.opts.maxHands) this.stop();
        break;
      }
      case "your_turn": {
        const st: YourTurn = {
          ...data, hand_id: data.hand_id ?? env.hand_id ?? "", seq: data.seq ?? env.seq ?? 0,
          table_id: tid, t_ms: env.t_ms ?? 0,
        };
        this.startDecision(st);
        break;
      }
      case "error": {
        const err = data as ApiError;
        this.errors.push(err);
        if (err.decision_id) this.answered.delete(err.decision_id); // the server re-sends your_turn with the same id
        await this.hook("onError", err);
        break;
      }
    }
  }

  private startDecision(st: YourTurn): void {
    if (this.answered.has(st.decision_id)) return;
    this.answered.add(st.decision_id);
    const p = this.decide(st).finally(() => this.inflight.delete(p));
    this.inflight.add(p);
  }

  private budgetMs(st: YourTurn): number | null {
    if (!st.deadline_ms) return null;
    const ref = st.t_ms || Date.now(); // server clock from the envelope when present: skew-proof
    return Math.max(0, st.deadline_ms - ref - this.opts.safetyMarginMs);
  }

  private async decide(st: YourTurn): Promise<void> {
    let action: Action | undefined;
    const budget = this.budgetMs(st);
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      const work = Promise.resolve().then(() => this.bot.act(st));
      work.catch(() => undefined);
      action = await (budget === null ? work : Promise.race([
        work,
        new Promise<never>((_, rej) => { timer = setTimeout(() => rej(new Error("act() exceeded the decision deadline")), budget); }),
      ]));
    } catch (e) {
      this.opts.log?.(`act failed (${e}); playing check/fold`);
    } finally {
      if (timer) clearTimeout(timer);
    }
    if (!action || typeof action.type !== "string") action = safeAction(st);
    else if (!this.bot.strict) action = legalize(st, action);
    const data: Record<string, unknown> = { hand_id: st.hand_id, decision_id: st.decision_id, type: action.type };
    if (action.to !== undefined && (action.type === "raise" || action.type === "bet" || action.type === "call")) data.to = action.to;
    try { this.send("act", data, st.table_id || undefined); }
    catch { this.answered.delete(st.decision_id); }
  }
}

/** Convenience: run `bot` until `maxHands`, `runner.stop()` or process exit. Resolves with the Runner (stats). */
export async function run(bot: BotLike, url: string, token: string, opts?: RunnerOptions): Promise<Runner> {
  const r = new Runner(bot, url, token, opts);
  await r.run();
  return r;
}
