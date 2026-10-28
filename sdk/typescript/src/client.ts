import type { Player, Seat, Table, TableConfig } from "./types.js";

export class ApiException extends Error {
  constructor(public status: number, public code: string, message = "", public details: Record<string, unknown> = {}) {
    super(`${status} ${code}: ${message}`);
  }
}

/** Thin REST client for /v1 (uses global `fetch`: Node >= 18 and browsers). `baseUrl` may be http(s):// or ws(s)://. */
export class ArenaClient {
  readonly baseUrl: string;
  constructor(baseUrl: string, public token?: string, private fetchImpl: typeof fetch = (...a) => fetch(...a)) {
    this.baseUrl = baseUrl.replace(/^ws/, "http").replace(/\/+$/, "");
  }

  async request<T = any>(method: string, path: string, body?: unknown, opts: { token?: string; query?: Record<string, string | number | undefined> } = {}): Promise<T> {
    let url = this.baseUrl + path;
    if (opts.query) {
      const q = new URLSearchParams();
      for (const [k, v] of Object.entries(opts.query)) if (v !== undefined) q.set(k, String(v));
      if ([...q].length) url += "?" + q;
    }
    const headers: Record<string, string> = { Accept: "application/json" };
    if (body !== undefined) headers["Content-Type"] = "application/json";
    const tok = opts.token ?? this.token;
    if (tok) headers.Authorization = `Bearer ${tok}`;
    const res = await this.fetchImpl(url, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
    const text = await res.text();
    if (!res.ok) {
      let err: any = { code: "internal", message: text.slice(0, 200) };
      try { err = JSON.parse(text).error ?? err; } catch { /* keep default */ }
      throw new ApiException(res.status, err.code, err.message, err.details);
    }
    return text ? JSON.parse(text) : (undefined as T);
  }

  /** POST /v1/players. The token is shown once; by default it becomes this client's token. */
  async registerBot(name: string, kind: "bot" | "human" = "bot", setToken = true): Promise<Player> {
    const p = await this.request<Player>("POST", "/v1/players", { name, kind });
    if (setToken && p.token) this.token = p.token;
    return p;
  }
  getPlayer(id: string) { return this.request<Player>("GET", `/v1/players/${id}`); }

  /** POST /v1/tables. Defaults: 6-max NLHE 50/100, 10000 stack, reset mode. */
  createTable(config: Partial<TableConfig> = {}): Promise<Table> {
    const cfg: TableConfig = {
      variant: "nlhe", max_seats: 6, small_blind: 50, big_blind: 100, ante: 0, straddle: false, starting_stack: 10000,
      stack_mode: "reset", decision_time_ms: 5000, time_bank_ms: 30000, timeout_action: "check_fold",
      rake: { percent: 0, cap: 0 }, min_players_to_deal: 6, odd_chip_rule: "first_left_of_button", show_mode: "standard", ...config,
    };
    return this.request<Table>("POST", "/v1/tables", cfg);
  }
  getTable(id: string) { return this.request<Table>("GET", `/v1/tables/${id}`); }
  joinTable(tableId: string, seat: number, playerId: string, token?: string) {
    return this.request<Seat>("POST", `/v1/tables/${tableId}/seats/${seat}`, { player_id: playerId }, { token });
  }
  leaveTable(tableId: string, seat: number) { return this.request<void>("DELETE", `/v1/tables/${tableId}/seats/${seat}`); }

  createMatch(body: { table_config: Partial<TableConfig>; participants: string[]; hands_planned: number; duplicate?: unknown; seed?: string }) {
    return this.request("POST", "/v1/matches", body);
  }
  matchAction(matchId: string, action: "start" | "pause" | "abort") { return this.request("POST", `/v1/matches/${matchId}/${action}`); }
  getMatch(id: string) { return this.request("GET", `/v1/matches/${id}`); }
  getHands(matchId: string, afterHandNo?: number, limit = 500) {
    return this.request<unknown[]>("GET", `/v1/matches/${matchId}/hands`, undefined, { query: { after_hand_no: afterHandNo, limit } });
  }
  getHand(id: string) { return this.request("GET", `/v1/hands/${id}`); }
  getHandEvents(id: string) { return this.request<unknown[]>("GET", `/v1/hands/${id}/events`); }
}

/** Accepts http(s)://host, ws(s)://host or a full ws(s)://host/v1/play URL; appends /v1/play and ?token=. */
export function wsUrl(url: string, token?: string): string {
  const u = new URL(url.replace(/^http/, "ws"));
  if (!u.pathname.replace(/\/+$/, "").endsWith("/v1/play")) u.pathname = u.pathname.replace(/\/+$/, "") + "/v1/play";
  if (token && !u.searchParams.has("token")) u.searchParams.set("token", token);
  return u.toString();
}
