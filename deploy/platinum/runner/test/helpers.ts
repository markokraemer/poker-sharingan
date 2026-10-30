// Shared test doubles: a scriptable fake sandbox backend and a fake arena HTTP server.
import http from "node:http";
import { gzipSync } from "node:zlib";
import type { AddressInfo } from "node:net";
import { loadConfig, type Config, type Env } from "../src/config.ts";
import type {
  CreateOpts, ExecResult, PlatinumClient, ProcessHandle, ProcessStatus, Sandbox, SandboxInfo, SandboxSpec, StartProcessOpts,
} from "../src/platinum/types.ts";

export const RUNNER_TOKEN = "runner-secret";
export const WEBHOOK_SECRET = "whsec_test_fake_secret";
export const AUTH = "svc-token";

export function testConfig(arenaUrl: string, extra: Env = {}): Config {
  return loadConfig({
    RUNNER_AUTH_TOKEN: AUTH,
    ARENA_URL: arenaUrl,
    ARENA_RUNNER_TOKEN: RUNNER_TOKEN,
    STRIPE_WEBHOOK_SECRET: WEBHOOK_SECRET,
    RUNNER_STORAGE: "memory",
    SANDBOX_BACKEND: "local",
    WORKER_SIGN_KEY: "test-signing-key",
    WORKER_KEY_ID: "key1",
    PRICE_USD_PER_1K_HANDS: "1",          // $1 per 1,000 hands: easy arithmetic
    POLL_INTERVAL_MS: "5",
    CONNECT_TIMEOUT_S: "1",
    ...extra,
  });
}

// ------------------------------------------------------------------------------------------------ fake sandbox backend

export class FakeProcess implements ProcessHandle {
  running = true;
  exitCode?: number;
  stdout = "";
  stderr = "";
  killed = false;
  opts: StartProcessOpts;
  constructor(opts: StartProcessOpts) { this.opts = opts; }
  exit(code: number, stderr = "") { this.running = false; this.exitCode = code; this.stderr += stderr; }
  async status(): Promise<ProcessStatus> { return this.running ? { running: true } : { running: false, exitCode: this.exitCode }; }
  async logs() { return { stdout: this.stdout, stderr: this.stderr }; }
  async kill() { this.killed = true; this.running = false; this.exitCode = 137; }
}

export interface Behavior {
  /** Return a result to override the default `exit 0` for a shell script. */
  sh?: (script: string) => ExecResult | undefined;
  /** Called when a process starts; typically schedules `p.exit(...)` and writes files. */
  onProcess?: (p: FakeProcess, sbx: FakeSandbox) => void;
  createError?: Error;
  deleteError?: Error;
  usage?: Record<string, unknown>;
}

export class FakeSandbox implements Sandbox {
  id: string;
  spec: SandboxSpec;
  files = new Map<string, Uint8Array>();
  fileModes = new Map<string, number | undefined>();
  shCalls: string[] = [];
  procs: FakeProcess[] = [];
  egress: string[][] = [];
  deleted = false;
  private b: Behavior;
  constructor(id: string, spec: SandboxSpec, b: Behavior) { this.id = id; this.spec = spec; this.b = b; }

  async sh(script: string): Promise<ExecResult> {
    this.shCalls.push(script);
    return this.b.sh?.(script) ?? { exitCode: 0, stdout: "", stderr: "" };
  }
  async writeFile(path: string, data: Uint8Array, opts: { mode?: number } = {}) { this.files.set(path, data); this.fileModes.set(path, opts.mode); }
  async readFile(path: string) {
    const f = this.files.get(path);
    if (!f) throw new Error(`no such file ${path}`);
    return f;
  }
  async startProcess(opts: StartProcessOpts) {
    const p = new FakeProcess(opts);
    this.procs.push(p);
    this.b.onProcess?.(p, this);
    return p;
  }
  async setEgress(allowOut: string[]) { this.egress.push(allowOut); }
  async usage() { return this.b.usage; }
  async delete() {
    if (this.b.deleteError) throw this.b.deleteError;
    this.deleted = true;
  }
}

export class FakePlatinum implements PlatinumClient {
  readonly kind = "local" as const;
  sandboxes: FakeSandbox[] = [];
  behavior: Behavior;
  private seq = 0;
  constructor(behavior: Behavior = {}) { this.behavior = behavior; }

  async createSandbox(spec: SandboxSpec, _opts?: CreateOpts): Promise<Sandbox> {
    if (this.behavior.createError) throw this.behavior.createError;
    const s = new FakeSandbox(`sbx${++this.seq}`, spec, this.behavior);
    this.sandboxes.push(s);
    return s;
  }
  async *listSandboxes(): AsyncIterable<SandboxInfo> {
    for (const s of this.sandboxes) if (!s.deleted) yield { id: s.id, metadata: s.spec.metadata, createdAtMs: 0 };
  }
  async deleteSandbox(id: string) { const s = this.sandboxes.find((x) => x.id === id); if (s) s.deleted = true; }
  get last(): FakeSandbox { return this.sandboxes[this.sandboxes.length - 1]!; }
  get live(): FakeSandbox[] { return this.sandboxes.filter((s) => !s.deleted); }
}

// ------------------------------------------------------------------------------------------------ fake arena

export interface ArenaCalls { path: string; method: string; auth?: string; body?: any }

export class FakeArena {
  server: http.Server;
  url = "";
  calls: ArenaCalls[] = [];
  /** connect mode: the run can start only once a "bot" has connected. */
  connected = false;
  /** connect mode: GET .../runs/:id returns `finished` after this many polls. */
  pollsToFinish = 3;
  handsPlayed?: number;           // override hands_done reported in the finished run / result ack
  resultStatus = 200;             // POST /v1/results answer
  resultVerified = true;
  startStatus?: number;           // force an error on run start
  private polls = 0;
  private runHands = 0;

  constructor() {
    this.server = http.createServer((req, res) => {
      const chunks: Buffer[] = [];
      req.on("data", (c) => chunks.push(c));
      req.on("end", () => {
        const raw = Buffer.concat(chunks).toString("utf8");
        let body: any;
        try { body = raw ? JSON.parse(raw) : undefined; } catch { body = raw; }
        const path = req.url ?? "";
        this.calls.push({ path, method: req.method ?? "", auth: req.headers.authorization, body });
        const send = (code: number, j: unknown) => { res.writeHead(code, { "content-type": "application/json" }); res.end(JSON.stringify(j)); };
        if (req.method === "GET" && path === "/v1/me") {
          const t = (req.headers.authorization ?? "").replace(/^Bearer /, "");
          if (t.startsWith("sk_user_")) return send(200, { player_id: `pl_${t.slice(8)}`, name: t.slice(8) });
          return send(401, { error: { code: "unauthorized", message: "bad token" } });
        }
        if (req.method === "POST" && path === "/v1/bots") return send(201, { player_id: "p_test", token: "sk_bot_token" });
        let m: RegExpMatchArray | null;
        if (req.method === "POST" && (m = path.match(/^\/v1\/benchmarks\/([\w-]+)\/runs$/))) {
          if (req.headers.authorization !== `Bearer ${RUNNER_TOKEN}`) return send(403, { error: { code: "forbidden", message: "runner token required" } });
          if (this.startStatus) return send(this.startStatus, { error: { code: "bad_request", message: "refused" } });
          if (!this.connected) return send(409, { error: { code: "not_connected", message: "connect first" } });
          this.polls = 0;
          this.runHands = Math.ceil((body?.hands ?? 600) / 6) * 6;
          return send(201, { run_id: "r_test", track: m[1], player_id: body?.player_id, status: "running", verified: body?.hosted === true, hands_planned: this.runHands, hands_done: 0, mbb_per_game: null, ci95_mbb: null });
        }
        if (req.method === "GET" && (m = path.match(/^\/v1\/benchmarks\/([\w-]+)\/runs\/r_test$/))) {
          this.polls++;
          const done = this.polls >= this.pollsToFinish;
          const played = this.handsPlayed ?? this.runHands;
          return send(200, {
            run_id: "r_test", track: m[1], player_id: "p_test", status: done ? "finished" : "running", verified: true, hands_planned: this.runHands,
            hands_done: done ? played : Math.floor((this.runHands / this.pollsToFinish) * this.polls), mbb_per_game: done ? 42.5 : null, ci95_mbb: done ? 8.1 : null,
            running_mbb_per_game: 40,
          });
        }
        if (req.method === "POST" && path.endsWith("/abort")) return send(200, { ok: true });
        if (req.method === "POST" && path === "/v1/results") {
          if (req.headers.authorization !== `Bearer ${RUNNER_TOKEN}`) return send(403, { error: { code: "forbidden", message: "runner token required" } });
          if (this.resultStatus !== 200) return send(this.resultStatus, { error: { code: "bad_signature", message: "signature mismatch" } });
          return send(201, { run_id: "r_worker", verified: this.resultVerified, status: "finished", hands_done: this.handsPlayed ?? body?.hands ?? 0, hands_total: body?.hands_planned ?? 0, mbb_per_game: body?.mbb_per_game ?? null, ci95_mbb: body?.ci95_mbb ?? null, forfeit_hands: body?.forfeit_hands ?? 0 });
        }
        send(404, { error: { code: "not_found", message: path } });
      });
    });
  }

  async start(): Promise<string> {
    await new Promise<void>((r) => this.server.listen(0, "127.0.0.1", r));
    this.url = `http://127.0.0.1:${(this.server.address() as AddressInfo).port}`;
    return this.url;
  }
  async stop() { await new Promise((r) => this.server.close(r)); }
  called(method: string, pathPrefix: string): ArenaCalls[] { return this.calls.filter((c) => c.method === method && c.path.startsWith(pathPrefix)); }
}

export async function until(cond: () => boolean | Promise<boolean>, ms = 3000, what = "condition"): Promise<void> {
  const t0 = Date.now();
  while (!(await cond())) {
    if (Date.now() - t0 > ms) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 5));
  }
}

/** A worker result.json shaped like `sharingan-arena run` writes it (schema sharingan-worker-result/1). */
export const workerResult = (over: Record<string, unknown> = {}) => JSON.stringify({
  schema: "sharingan-worker-result/1", run_id: "w_test0001", track: "ladder-v1", hands: 600, hands_planned: 600, mbb_per_game: 12.5, ci95_mbb: 3.2,
  forfeit_hands: 0, signature: { alg: "HMAC-SHA256", key_id: "key1", value: "deadbeef" }, ...over,
});

/** A real .tar.gz (ustar) holding `files`, as base64: what POST /v1/jobs now requires. */
export function makeTgz(files: Record<string, string> = { "bot.py": "print('hi')\n" }): Buffer {
  const blocks: Buffer[] = [];
  for (const [name, content] of Object.entries(files)) {
    const data = Buffer.from(content);
    const h = Buffer.alloc(512);
    h.write(name, 0, 100); h.write("0000644\0", 100); h.write("0000000\0", 108); h.write("0000000\0", 116);
    h.write(data.length.toString(8).padStart(11, "0") + "\0", 124); h.write("00000000000\0", 136);
    h.fill(0x20, 148, 156); h.write("0", 156); h.write("ustar\0", 257); h.write("00", 263);
    let sum = 0; for (const b of h) sum += b;
    h.write(sum.toString(8).padStart(6, "0") + "\0 ", 148);
    blocks.push(h, data, Buffer.alloc((512 - (data.length % 512)) % 512));
  }
  blocks.push(Buffer.alloc(1024));
  return gzipSync(Buffer.concat(blocks));
}
export const tgzB64 = (files?: Record<string, string>): string => makeTgz(files).toString("base64");
