// HTTP front for the runner. UNTESTED.
//   POST /jobs            {userId,botName,language,artifactTgzBase64,entry,track,hands,lanes?,idempotencyKey} -> 202 {id}
//   GET  /jobs/:id        status, progress, result, cost
//   POST /jobs/:id/abort
// Auth: Authorization: Bearer $RUNNER_AUTH_TOKEN (called only by our own web/billing backend, never by players directly).
// Credits are reserved by the caller BEFORE POST /jobs and settled from GET /jobs/:id (or the webhook) after.
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { randomBytes, timingSafeEqual } from "node:crypto";
import { config } from "./config.ts";
import { runJob, requestAbort, reapOrphans } from "./job.ts";
import { store, type JobInput } from "./store.ts";

const MAX_BODY = Math.ceil(config.maxArtifactBytes * 1.4) + 64 * 1024;
const TRACKS = new Set(["ladder-v1", "sharingan-1"]);
let running = 0;
const waiting: Array<() => void> = [];
async function slot<T>(fn: () => Promise<T>): Promise<T> {
  if (running >= config.maxConcurrentRuns) await new Promise<void>((r) => waiting.push(r));
  running++;
  try { return await fn(); } finally { running--; waiting.shift()?.(); }
}

function authed(req: IncomingMessage): boolean {
  const got = Buffer.from((req.headers.authorization ?? "").replace(/^Bearer /, ""));
  const want = Buffer.from(config.authToken);
  return got.length === want.length && timingSafeEqual(got, want);
}
const send = (res: ServerResponse, code: number, body: unknown) => {
  res.writeHead(code, { "content-type": "application/json" });
  res.end(JSON.stringify(body));
};
async function readJson(req: IncomingMessage): Promise<any> {
  const chunks: Buffer[] = []; let n = 0;
  for await (const c of req) { n += (c as Buffer).length; if (n > MAX_BODY) throw new Error("body too large"); chunks.push(c as Buffer); }
  return JSON.parse(Buffer.concat(chunks).toString("utf8"));
}
function validate(b: any): JobInput {
  const bad = (m: string) => { throw new Error(m); };
  if (!b || typeof b !== "object") bad("body");
  for (const k of ["userId", "botName", "artifactTgzBase64", "entry", "track", "idempotencyKey"]) if (typeof b[k] !== "string" || !b[k]) bad(`${k} required`);
  if (!["python", "typescript"].includes(b.language)) bad("language must be python|typescript");
  if (!TRACKS.has(b.track)) bad("unknown track");
  if (!/^[\w./-]{1,200}$/.test(b.entry) || b.entry.includes("..") || b.entry.startsWith("/")) bad("bad entry path");
  if (b.requirementsFile && (!/^[\w./-]{1,200}$/.test(b.requirementsFile) || b.requirementsFile.includes(".."))) bad("bad requirementsFile");
  const hands = Number(b.hands ?? 20000);
  if (!Number.isInteger(hands) || hands < 600 || hands > 200_000) bad("hands must be 600..200000");
  return { ...b, hands };
}

createServer(async (req, res) => {
  try {
    if (req.url === "/healthz") return send(res, 200, { ok: true, running });
    if (!authed(req)) return send(res, 401, { error: "unauthorized" });
    const url = new URL(req.url!, "http://x");
    const m = url.pathname.match(/^\/jobs(?:\/([\w-]+))?(\/abort)?$/);
    if (!m) return send(res, 404, { error: "not found" });

    if (req.method === "POST" && !m[1]) {
      const input = validate(await readJson(req));
      const existing = store.byKey(input.idempotencyKey);
      if (existing) return send(res, 200, { id: existing.id, deduped: true });
      const id = `job_${randomBytes(8).toString("hex")}`;
      const { artifactTgzBase64: _drop, ...meta } = input;
      store.put({ id, input: meta, status: "queued", createdAt: Date.now() });
      void slot(() => runJob(id, input)).catch((e) => store.patch(id, { status: "failed", error: String(e) }));
      return send(res, 202, { id });
    }
    if (req.method === "GET" && m[1]) {
      const j = store.get(m[1]);
      return j ? send(res, 200, j) : send(res, 404, { error: "no such job" });
    }
    if (req.method === "POST" && m[1] && m[2]) {
      if (!store.get(m[1])) return send(res, 404, { error: "no such job" });
      requestAbort(m[1]);
      return send(res, 202, { ok: true });
    }
    send(res, 405, { error: "method not allowed" });
  } catch (e) {
    send(res, 400, { error: String((e as Error).message ?? e) });
  }
}).listen(config.port, () => console.log(`runner listening on :${config.port}`));

void reapOrphans().catch(() => {});
setInterval(() => void reapOrphans().catch((e) => console.error("reap", e)), 5 * 60_000);
