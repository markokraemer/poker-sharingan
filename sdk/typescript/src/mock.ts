/**
 * In-process mock arena implementing just enough of docs/TABLE_API.md to test bots (Node only; needs `ws`).
 * REST + WebSocket /v1/play share one port. Each hand is scripted: the bot sits in seat 0 (UTG), gets a
 * preflop `your_turn` (facing the BB) and, unless it folds, a flop `your_turn` (checking is free).
 * Features: idempotent `act`, `stale_decision`, `illegal_action`/`bad_amount` (re-sends the same `your_turn`),
 * `table_seq`/`seq`, `hello{resume}` -> `table_snapshot` + re-sent pending `your_turn`, forced disconnects.
 */
import { createServer, type Server } from "node:http";
import { randomBytes } from "node:crypto";
import type { AddressInfo } from "node:net";
import { WebSocketServer, type WebSocket } from "ws";

const SB = 50, BB = 100, STACK = 10000;
const HOLE = ["Ah", "Kd"], FLOP = ["7c", "8d", "2s"];
const rid = (n = 4) => randomBytes(n).toString("hex");

export interface MockOptions {
  hands?: number;
  /** 1-based counts of your_turn sends after which the socket is dropped (once each). */
  dropOnTurns?: number[];
  decisionMs?: number;
  /** skip one table_seq number when it reaches this value (tests gap resync) */
  emitGapAt?: number;
}

export class MockArena {
  httpUrl = "";
  wsUrl = "";
  players = new Map<string, any>();
  tokens = new Map<string, string>();
  tables = new Map<string, any>();
  received: any[] = [];
  acts: any[] = [];
  errorsSent: string[] = [];
  handEnds: any[] = [];
  turnsSent = 0;
  connections = 0;
  handNo = 0;
  phase: "idle" | "preflop" | "flop" | "finished" = "idle";
  pending: any = null;
  private answered = new Set<string>();
  private tableId = "t1";
  private tableSeq = 0;
  private seq = 0;
  private x = 0;
  private gapDone = false;
  private history: any[] = [];
  private playerId = "p0";
  private drops: Set<number>;
  private server!: Server;
  private wss!: WebSocketServer;
  private doneResolvers: Array<() => void> = [];
  readonly hands: number;
  private decisionMs: number;
  private emitGapAt?: number;

  constructor(o: MockOptions = {}) {
    this.hands = o.hands ?? 3;
    this.drops = new Set(o.dropOnTurns ?? []);
    this.decisionMs = o.decisionMs ?? 5000;
    this.emitGapAt = o.emitGapAt;
  }

  static async start(o: MockOptions = {}): Promise<MockArena> { const m = new MockArena(o); await m.listen(); return m; }

  async listen(): Promise<void> {
    this.server = createServer((req, res) => this.rest(req, res));
    this.wss = new WebSocketServer({ server: this.server, path: "/v1/play" });
    this.wss.on("connection", (ws, req) => this.onConnection(ws, req.url ?? ""));
    await new Promise<void>((r) => this.server.listen(0, "127.0.0.1", r));
    const port = (this.server.address() as AddressInfo).port;
    this.httpUrl = `http://127.0.0.1:${port}`;
    this.wsUrl = `ws://127.0.0.1:${port}/v1/play`;
  }

  async close(): Promise<void> {
    for (const c of this.wss.clients) c.terminate();
    await new Promise<void>((r) => this.wss.close(() => r()));
    await new Promise<void>((r) => { this.server.close(() => r()); this.server.closeAllConnections?.(); });
  }

  /** Resolves when the scripted `hands` have all ended. */
  finished(): Promise<void> {
    return this.phase === "finished" ? Promise.resolve() : new Promise((r) => this.doneResolvers.push(r));
  }

  // ------------------------------------------------------------------ REST
  private rest(req: import("node:http").IncomingMessage, res: import("node:http").ServerResponse): void {
    const chunks: Buffer[] = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => {
      const body = chunks.length ? JSON.parse(Buffer.concat(chunks).toString()) : {};
      const send = (s: number, o: unknown) => res.writeHead(s, { "Content-Type": "application/json" }).end(JSON.stringify(o));
      const err = (s: number, code: string) => send(s, { error: { code, message: code, details: {} } });
      const path = (req.url ?? "").split("?")[0]!, parts = path.split("/").filter(Boolean), m = req.method;
      if (m === "POST" && path === "/v1/players") {
        const pid = "p-" + rid(), tok = "tok-" + rid(16);
        const rec = { player_id: pid, name: body.name, kind: body.kind ?? "bot", created_at: "2026-01-01T00:00:00Z" };
        this.players.set(pid, rec); this.tokens.set(tok, pid);
        return send(201, { ...rec, token: tok });
      }
      const auth = (req.headers.authorization ?? "").replace("Bearer ", "");
      if (!this.tokens.has(auth)) return err(401, "unauthorized");
      if (m === "POST" && path === "/v1/tables") {
        const tid = "t-" + rid(3);
        const t = { table_id: tid, config: body, status: "waiting", button_seat: null, hand_id: null, table_seq: 0,
          seats: Array.from({ length: 6 }, (_, i) => ({ seat: i, player_id: null, name: null, stack: 0, status: "empty" })) };
        this.tables.set(tid, t);
        return send(201, t);
      }
      if (parts[1] === "tables" && parts.length === 3 && m === "GET") { const t = this.tables.get(parts[2]!); return t ? send(200, t) : err(404, "not_found"); }
      if (parts[1] === "tables" && parts[3] === "seats" && parts.length === 5) {
        const t = this.tables.get(parts[2]!); if (!t) return err(404, "not_found");
        const seat = t.seats[Number(parts[4])];
        if (m === "POST") {
          if (seat.player_id) return err(409, "seat_taken");
          const p = this.players.get(body.player_id);
          Object.assign(seat, { player_id: p.player_id, name: p.name, stack: STACK, status: "seated" });
          return send(200, seat);
        }
        if (m === "DELETE") { Object.assign(seat, { player_id: null, name: null, stack: 0, status: "empty" }); return send(200, seat); }
      }
      err(404, "not_found");
    });
  }

  // ------------------------------------------------------------------ WS
  private env(type: string, data: any, hand = true, withSeq = true): string {
    this.tableSeq++;
    if (this.emitGapAt && this.tableSeq === this.emitGapAt && !this.gapDone) { this.gapDone = true; this.tableSeq++; }
    const e: any = { v: 1, type, table_id: this.tableId, table_seq: this.tableSeq, t_ms: Date.now(), data };
    if (hand && this.handNo) {
      e.hand_id = `h-${this.handNo}`;
      if (withSeq) { this.seq++; e.seq = this.seq; data.seq ??= this.seq; }
    }
    return JSON.stringify(e);
  }

  private onConnection(ws: WebSocket, url: string): void {
    const token = new URL(url, "http://x").searchParams.get("token") ?? "";
    if (!this.tokens.has(token) && !token.startsWith("any")) { ws.close(4401, "unauthorized"); return; }
    this.connections++;
    this.playerId = this.tokens.get(token) ?? "p0";
    let chain = Promise.resolve();
    ws.on("message", (raw) => {
      const msg = JSON.parse(raw.toString());
      this.received.push(msg);
      chain = chain.then(async () => {
        if (msg.type === "hello") await this.onHello(ws);
        else if (msg.type === "ping") ws.send(JSON.stringify({ v: 1, type: "pong", t_ms: Date.now(), data: {} }));
        else if (msg.type === "act") await this.onAct(ws, msg.data);
      }).catch(() => undefined);
    });
  }

  private async onHello(ws: WebSocket): Promise<void> {
    ws.send(JSON.stringify({ v: 1, type: "welcome", t_ms: Date.now(), data: { player_id: this.playerId, name: "bot", server_time: Date.now(), tables: [this.tableId], protocol: 1 } }));
    const seats = Array.from({ length: 6 }, (_, i) => ({ seat: i, player_id: i === 0 ? this.playerId : `other${i}`, name: `s${i}`, stack: STACK, status: "seated" }));
    ws.send(JSON.stringify({ v: 1, type: "table_snapshot", table_id: this.tableId, table_seq: this.tableSeq, t_ms: Date.now(),
      data: { table_id: this.tableId, config: { small_blind: SB, big_blind: BB, starting_stack: STACK }, status: "running", seats, button_seat: 3, hand_id: this.handNo ? `h-${this.handNo}` : null, table_seq: this.tableSeq, hand: null } }));
    if (this.phase === "idle") this.startHand(ws);
    else if (this.pending) this.sendTurn(ws);
  }

  private players_(contrib: Record<number, number>) {
    return Array.from({ length: 6 }, (_, s) => { const c = contrib[s] ?? 0; return { seat: s, name: `s${s}`, stack_start: STACK, stack: STACK - c, street_contrib: c, hand_contrib: c, status: "active" }; });
  }

  private actionEv(seat: number, type: string, to: number, added: number, stack: number, pot: number, street: string, forced = false, next: number | null = null) {
    this.seq++;
    return { hand_id: `h-${this.handNo}`, seq: this.seq, action_id: `a-${this.handNo}-${this.seq}`, street, seat, type, to, added, all_in: false, forced, timeout: false, stack, pot, current_bet: type === "fold" ? BB : to, min_raise_to: 2 * BB, next_to_act: next };
  }

  private startHand(ws: WebSocket): void {
    this.handNo++; this.seq = 0; this.phase = "preflop"; this.answered.clear();
    const hs = { hand_id: `h-${this.handNo}`, hand_no: this.handNo, match_id: "m1", deal_no: null, rotation: null, button_seat: 3, sb_seat: 4, bb_seat: 5, small_blind: SB, big_blind: BB, ante: 0,
      players: Array.from({ length: 6 }, (_, i) => ({ seat: i, player_id: i === 0 ? this.playerId : `other${i}`, name: `s${i}`, stack_start: STACK, in_hand: true })), deck_commitment: "00".repeat(32) };
    ws.send(this.env("hand_start", hs, true, false));
    ws.send(this.env("hole_cards", { hand_id: `h-${this.handNo}`, seat: 0, cards: HOLE }, true, false));
    this.history = [this.actionEv(4, "post_sb", SB, SB, STACK - SB, SB, "preflop", true, 5), this.actionEv(5, "post_bb", BB, BB, STACK - BB, SB + BB, "preflop", true, 0)];
    for (const ev of this.history) ws.send(this.env("action", { ...ev }, true, false));
    this.pending = this.turn("preflop", [], { 4: SB, 5: BB }, SB + BB, { to: BB, added: BB, all_in: false }, false, { min_to: 2 * BB, max_to: STACK });
    this.sendTurn(ws);
  }

  private turn(street: string, board: string[], contrib: Record<number, number>, pot: number, call: any, check: boolean, raise: any) {
    this.seq++;
    return { hand_id: `h-${this.handNo}`, seq: this.seq, decision_id: `d-${this.handNo}-${this.seq}`, seat: 0, street, hole_cards: HOLE, board, history: [...this.history],
      players: this.players_(contrib), button_seat: 3, small_blind: SB, big_blind: BB, pot, pots: [{ amount: pot, eligible_seats: [0, 4, 5] }],
      legal: { fold: true, check, call, raise }, deadline_ms: Date.now() + this.decisionMs, time_bank_ms: 30000 };
  }

  private sendTurn(ws: WebSocket): void {
    this.pending.deadline_ms = Date.now() + this.decisionMs;
    this.turnsSent++;
    const e = JSON.parse(this.env("your_turn", { ...this.pending }, true, false));
    e.seq = this.pending.seq;
    ws.send(JSON.stringify(e));
    if (this.drops.delete(this.turnsSent)) ws.close();
  }

  private sendErr(ws: WebSocket, code: string, decisionId?: string, details: object = {}): void {
    this.errorsSent.push(code);
    ws.send(this.env("error", { code, message: code, details, ...(decisionId ? { decision_id: decisionId } : {}) }, false));
  }

  private async onAct(ws: WebSocket, d: any): Promise<void> {
    const p = this.pending;
    if (!p || d.hand_id !== p.hand_id) return this.sendErr(ws, "not_your_turn");
    if (this.answered.has(d.decision_id)) return;
    if (d.decision_id !== p.decision_id) return this.sendErr(ws, "stale_decision", d.decision_id);
    let t: string = d.type; const legal = p.legal;
    const ok = (t === "fold" && legal.fold) || (t === "check" && legal.check) || (t === "call" && legal.call) || ((t === "raise" || t === "bet") && legal.raise);
    if (!ok) { this.sendErr(ws, "illegal_action", p.decision_id); return this.sendTurn(ws); }
    if ((t === "raise" || t === "bet") && !(Number.isInteger(d.to) && d.to >= legal.raise.min_to && d.to <= legal.raise.max_to)) {
      this.sendErr(ws, "bad_amount", p.decision_id, { min_to: legal.raise.min_to, max_to: legal.raise.max_to });
      return this.sendTurn(ws);
    }
    this.answered.add(p.decision_id); this.acts.push(d); this.pending = null;
    if (t === "fold" && legal.check) t = "check";
    const to = t === "call" ? legal.call.to : t === "raise" || t === "bet" ? d.to : 0;
    const ev = this.actionEv(0, t, to, to, STACK - to, p.pot + to, p.street);
    this.history.push(ev);
    ws.send(this.env("action", { ...ev }, true, false));
    if (p.street === "preflop") {
      if (t === "fold") return this.endHand(ws, "fold", 0);
      this.x = to;
      const pot = 2 * this.x + SB;
      ws.send(this.env("street", { hand_id: `h-${this.handNo}`, street: "flop", board: FLOP, new_cards: FLOP, pot, pots: [{ amount: pot, eligible_seats: [0, 5] }] }, true, false));
      this.phase = "flop";
      this.pending = this.turn("flop", FLOP, {}, pot, null, true, { min_to: BB, max_to: STACK - this.x });
      this.sendTurn(ws);
    } else if (t === "raise" || t === "bet") this.endHand(ws, "fold", this.x + SB);
    else { const win = this.handNo % 2 === 1; this.endHand(ws, "showdown", win ? this.x : -this.x); }
  }

  private endHand(ws: WebSocket, endedBy: string, botNet: number): void {
    const bbNet = SB - botNet;
    const results = [{ seat: 0, player_id: this.playerId, net: botNet, stack_end: STACK + botNet }, { seat: 4, player_id: "other4", net: -SB, stack_end: STACK - SB }, { seat: 5, player_id: "other5", net: bbNet, stack_end: STACK + bbNet }];
    const data = { hand_id: `h-${this.handNo}`, board: [...FLOP, "3h", "9c"],
      showdown: endedBy === "showdown" ? [{ seat: 0, cards: HOLE, shown: true, hand_rank: "high card", best5: [] }] : [],
      hole_cards: Object.fromEntries(Array.from({ length: 6 }, (_, i) => [String(i), i === 0 ? HOLE : ["2c", "3d"]])),
      pots: [{ amount: 2 * this.x + SB, eligible_seats: [0, 5], winners: [botNet > 0 ? 0 : 5], share: [2 * this.x + SB] }],
      results, deck_reveal: { deck: [], salt: "x" }, ended_by: endedBy };
    this.handEnds.push(data);
    ws.send(this.env("hand_end", data, true, true));
    this.phase = "idle";
    if (this.handNo >= this.hands) { this.phase = "finished"; this.doneResolvers.splice(0).forEach((r) => r()); return; }
    this.startHand(ws);
  }
}
