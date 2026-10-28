// Minimal Arena REST client (docs/TABLE_API.md, arena/server/README.md). UNTESTED.
import { config } from "./config.ts";

export interface BenchRun {
  run_id: string;
  track: string;
  player_id: string;
  match_id: string;
  status: "running" | "finished" | "aborted" | string;
  verified: boolean;
  hands_planned: number;
  hands_done: number;
  mbb_per_game: number | null;
  ci95_mbb: number | null;
  running_mbb_per_game?: number | null;
  running_ci95_mbb?: number | null;
  method?: string | null;
}

export class ArenaError extends Error {
  constructor(public status: number, public code: string, message: string) { super(`${status} ${code}: ${message}`); }
}

async function call<T>(method: string, path: string, token: string | undefined, body?: unknown): Promise<T> {
  const res = await fetch(new URL(path, config.arenaUrl), {
    method,
    headers: { "content-type": "application/json", ...(token ? { authorization: `Bearer ${token}` } : {}) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  const json = text ? JSON.parse(text) : {};
  if (!res.ok) throw new ArenaError(res.status, json?.error?.code ?? json?.code ?? "error", json?.error?.message ?? text.slice(0, 200));
  return json as T;
}

/** Register a fresh bot identity for this job. The token is shown once and is handed only to the sandbox + this process. */
export async function registerBot(name: string, owner: string): Promise<{ playerId: string; token: string }> {
  const r = await call<{ player_id: string; token: string }>("POST", "/v1/bots", config.arenaAdminToken, { name, kind: "bot", owner });
  return { playerId: r.player_id, token: r.token };
}

/** Start the benchmark run with the BOT's own token (a bot may only run itself). 409 not_connected until its websocket is up. */
export function startRun(track: string, botToken: string, opts: { hands: number; lanes?: number }): Promise<BenchRun> {
  return call("POST", `/v1/benchmarks/${encodeURIComponent(track)}/runs`, botToken, { hands: opts.hands, lanes: opts.lanes });
}
export function getRun(track: string, runId: string): Promise<BenchRun> {
  return call("GET", `/v1/benchmarks/${encodeURIComponent(track)}/runs/${encodeURIComponent(runId)}`, undefined);
}
export function abortRun(track: string, runId: string, botToken: string): Promise<unknown> {
  return call("POST", `/v1/benchmarks/${encodeURIComponent(track)}/runs/${encodeURIComponent(runId)}/abort`, botToken, {});
}
export function listTracks(): Promise<{ id?: string; track?: string; available: boolean }[]> {
  return call("GET", "/v1/benchmarks", undefined);
}
