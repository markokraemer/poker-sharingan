// A small client for the Docker Engine API (v1.41 subset). Podman serves the same API on its compat socket
// (`podman system service`, rootless: $XDG_RUNTIME_DIR/podman/podman.sock), so one client covers both engines.
import http from "node:http";

export class EngineError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(`engine ${status}: ${message}`);
    this.status = status;
  }
}

export interface EngineResponse { status: number; headers: http.IncomingHttpHeaders; body: Buffer }

export interface EngineOpts {
  /** unix:///path/to.sock or tcp://host:port (http://host:port also accepted). */
  host: string;
  apiVersion?: string;
  requestTimeoutMs?: number;
}

export class Engine {
  private readonly target: { socketPath: string } | { host: string; port: number };
  private readonly prefix: string;
  private readonly timeoutMs: number;

  constructor(opts: EngineOpts) {
    this.prefix = `/v${opts.apiVersion ?? "1.41"}`;
    this.timeoutMs = opts.requestTimeoutMs ?? 60_000;
    const h = opts.host;
    if (h.startsWith("unix://")) this.target = { socketPath: h.slice("unix://".length) };
    else {
      const u = new URL(h.replace(/^tcp:/, "http:"));
      this.target = { host: u.hostname, port: Number(u.port || 80) };
    }
  }

  request(method: string, path: string, opts: { json?: unknown; body?: Buffer; contentType?: string; timeoutMs?: number } = {}): Promise<EngineResponse> {
    return new Promise((resolve, reject) => {
      const payload = opts.body ?? (opts.json !== undefined ? Buffer.from(JSON.stringify(opts.json)) : undefined);
      const headers: Record<string, string | number> = {};
      if (payload) {
        headers["content-length"] = payload.length;
        headers["content-type"] = opts.contentType ?? "application/json";
      }
      const req = http.request({ ...this.target, method, path: this.prefix + path, headers }, (res) => {
        const chunks: Buffer[] = [];
        res.on("data", (c: Buffer) => chunks.push(c));
        res.on("end", () => resolve({ status: res.statusCode ?? 0, headers: res.headers, body: Buffer.concat(chunks) }));
        res.on("error", reject);
      });
      req.setTimeout(opts.timeoutMs ?? this.timeoutMs, () => req.destroy(new Error(`engine request timed out: ${method} ${path}`)));
      req.on("error", reject);
      req.end(payload);
    });
  }

  /** Request that must succeed (2xx); returns parsed JSON (or undefined for an empty body). */
  async json<T = unknown>(method: string, path: string, opts: { json?: unknown; ok?: number[]; timeoutMs?: number } = {}): Promise<T> {
    const r = await this.request(method, path, { json: opts.json, timeoutMs: opts.timeoutMs });
    const ok = opts.ok ?? [200, 201, 204];
    if (!ok.includes(r.status)) throw new EngineError(r.status, errText(r.body));
    const text = r.body.toString("utf8");
    return (text ? JSON.parse(text) : undefined) as T;
  }
}

function errText(body: Buffer): string {
  const t = body.toString("utf8");
  try { const j = JSON.parse(t); return String(j.message ?? t); } catch { return t.slice(0, 300); }
}

// ---- stream multiplexing (non-TTY exec / attach): 8-byte frame header [type,0,0,0,len(be32)] + payload ----

export function demux(buf: Buffer): { stdout: Buffer; stderr: Buffer } {
  const out: Buffer[] = [];
  const err: Buffer[] = [];
  let i = 0;
  while (i + 8 <= buf.length) {
    const type = buf[i]!;
    const len = buf.readUInt32BE(i + 4);
    const end = Math.min(buf.length, i + 8 + len);
    (type === 2 ? err : out).push(buf.subarray(i + 8, end));
    i = end;
  }
  return { stdout: Buffer.concat(out), stderr: Buffer.concat(err) };
}

// ---- minimal tar (ustar) writer for PUT /containers/{id}/archive ----

export function tarFile(name: string, data: Uint8Array, mode = 0o644, mtime = Math.floor(Date.now() / 1000)): Buffer {
  if (Buffer.byteLength(name) > 99) throw new Error("tar name too long");
  const header = Buffer.alloc(512);
  header.write(name, 0, "utf8");
  header.write(octal(mode, 7), 100, "ascii");
  header.write(octal(0, 7), 108, "ascii");          // uid
  header.write(octal(0, 7), 116, "ascii");          // gid
  header.write(octal(data.length, 11), 124, "ascii");
  header.write(octal(mtime, 11), 136, "ascii");
  header.write("        ", 148, "ascii");           // checksum placeholder (8 spaces)
  header.write("0", 156, "ascii");                  // regular file
  header.write("ustar\0", 257, "ascii");
  header.write("00", 263, "ascii");
  let sum = 0;
  for (const b of header) sum += b;
  header.write(octal(sum, 6) + "\0 ", 148, "ascii");
  const pad = (512 - (data.length % 512)) % 512;
  return Buffer.concat([header, Buffer.from(data), Buffer.alloc(pad), Buffer.alloc(1024)]);
}
const octal = (n: number, width: number): string => n.toString(8).padStart(width, "0");
