// Arena REST client (docs/TABLE_API.md 12b items 11-16, arena/server/README.md).
//
// Auth model used by the runner:
//   * ARENA_RUNNER_TOKEN: start/abort hosted (verified) benchmark runs, attest hosted bots, POST signed worker results.
//     It is NOT an admin token: the arena answers 403 to the runner everywhere else.
//   * bot registration needs no token while ARENA_OPEN_REGISTRATION=true; set ARENA_ADMIN_TOKEN on the runner only if
//     registration is closed.
import type { Config } from "./config.ts";

export interface BenchRun {
  run_id: string;
  track: string;
  player_id: string;
  match_id?: string;
  status: "running" | "finished" | "aborted" | string;
  verified: boolean;
  hosted?: boolean;
  hands_planned?: number;
  hands_total?: number;
  hands_done: number;
  mbb_per_game: number | null;
  ci95_mbb: number | null;
  running_mbb_per_game?: number | null;
  running_ci95_mbb?: number | null;
  method?: string | null;
}

/** The arena's answer to POST /v1/results (worker mode). Fields are read leniently; see worker.ts. */
export interface ResultAck {
  run_id?: string;
  verified?: boolean;
  status?: string;
  hands_done?: number;
  hands?: number;
  mbb_per_game?: number | null;
  ci95_mbb?: number | null;
  [k: string]: unknown;
}

export class ArenaError extends Error {
  status: number;
  code: string;
  constructor(status: number, code: string, message: string) {
    super(`arena ${status} ${code}: ${message}`);
    this.status = status;
    this.code = code;
  }
}

export class ArenaClient {
  private readonly cfg: Config["arena"];
  private readonly doFetch: typeof fetch;

  constructor(cfg: Config["arena"], doFetch: typeof fetch = fetch) {
    this.cfg = cfg;
    this.doFetch = doFetch;
  }

  private async call<T>(method: string, path: string, token: string | undefined, body?: string | Uint8Array | object): Promise<T> {
    const raw = body === undefined ? undefined : typeof body === "string" || body instanceof Uint8Array ? body : JSON.stringify(body);
    const res = await this.doFetch(new URL(path, this.cfg.url), {
      method,
      headers: { "content-type": "application/json", ...(token ? { authorization: `Bearer ${token}` } : {}) },
      body: raw as BodyInit | undefined,
      signal: AbortSignal.timeout(30_000),
    });
    const text = await res.text();
    let json: any = {};
    try { json = text ? JSON.parse(text) : {}; } catch { json = { error: { message: text.slice(0, 200) } }; }
    if (!res.ok) throw new ArenaError(res.status, json?.error?.code ?? json?.code ?? "error", json?.error?.message ?? json?.message ?? text.slice(0, 200));
    return json as T;
  }

  /** A fresh bot identity for one job. The token is shown once. */
  async registerBot(name: string, owner: string): Promise<{ playerId: string; token: string }> {
    const r = await this.call<{ player_id: string; token: string }>("POST", "/v1/bots", this.cfg.adminToken, { name, kind: "bot", owner });
    return { playerId: r.player_id, token: r.token };
  }

  /** Connect mode: start the hosted run for a bot that is already connected to /v1/play. 409 not_connected until it is. */
  startHostedRun(track: string, playerId: string, opts: { hands: number; lanes?: number }): Promise<BenchRun> {
    return this.call("POST", `/v1/benchmarks/${encodeURIComponent(track)}/runs`, this.cfg.runnerToken, {
      player_id: playerId, hands: opts.hands, lanes: opts.lanes, hosted: true,
    });
  }

  getRun(track: string, runId: string): Promise<BenchRun> {
    return this.call("GET", `/v1/benchmarks/${encodeURIComponent(track)}/runs/${encodeURIComponent(runId)}`, undefined);
  }

  abortRun(track: string, runId: string): Promise<unknown> {
    return this.call("POST", `/v1/benchmarks/${encodeURIComponent(track)}/runs/${encodeURIComponent(runId)}/abort`, this.cfg.runnerToken, {});
  }

  /** Worker mode: submit the worker's signed result.json verbatim (the signature covers the exact bytes). */
  postResult(signedBody: string | Uint8Array): Promise<ResultAck> {
    return this.call("POST", "/v1/results", this.cfg.runnerToken, signedBody);
  }

  async tracks(): Promise<Array<{ track?: string; id?: string; available?: boolean }>> {
    const r = await this.call<unknown>("GET", "/v1/benchmarks", undefined);
    return (Array.isArray(r) ? r : (r as { tracks?: unknown[] }).tracks ?? []) as Array<{ track?: string; id?: string; available?: boolean }>;
  }
}
