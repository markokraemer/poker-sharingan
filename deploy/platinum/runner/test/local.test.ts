// The local sandbox backend against a fake Docker Engine API on a unix socket: verifies the wire protocol the runner
// speaks (the same calls podman's compat socket serves; the real thing is exercised by deploy/local/e2e.sh).
import assert from "node:assert/strict";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { after, before, test } from "node:test";
import { demux, tarFile } from "../src/platinum/engine.ts";
import { LocalPlatinum } from "../src/platinum/local.ts";
import type { SandboxSpec } from "../src/platinum/types.ts";

interface Rec { method: string; path: string; query: URLSearchParams; json?: any; raw: Buffer }

let dir: string;
let server: http.Server;
let sock: string;
const rec: Rec[] = [];
let execs = new Map<string, { cmd: string[]; detach?: boolean }>();
let execSeq = 0;
let containers: Array<{ Id: string; Created: number; Labels: Record<string, string> }> = [];
let failConnect = "";

const frame = (type: number, s: string) => {
  const b = Buffer.from(s);
  const h = Buffer.alloc(8);
  h[0] = type;
  h.writeUInt32BE(b.length, 4);
  return Buffer.concat([h, b]);
};

before(async () => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), "fake-engine-"));
  sock = path.join(dir, "engine.sock");
  server = http.createServer((req, res) => {
    const chunks: Buffer[] = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => {
      const raw = Buffer.concat(chunks);
      const u = new URL(req.url!, "http://x");
      const p = u.pathname.replace(/^\/v1\.41/, "");
      let json: any;
      if ((req.headers["content-type"] ?? "").includes("json") && raw.length) json = JSON.parse(raw.toString());
      rec.push({ method: req.method!, path: p, query: u.searchParams, json, raw });
      const out = (code: number, body?: unknown, headers: Record<string, string> = { "content-type": "application/json" }) => {
        res.writeHead(code, headers);
        res.end(body === undefined ? undefined : typeof body === "string" || Buffer.isBuffer(body) ? body : JSON.stringify(body));
      };
      let m: RegExpMatchArray | null;
      if (req.method === "POST" && p === "/containers/create") {
        const id = `c${containers.length + 1}`;
        containers.push({ Id: id, Created: 1_700_000_000, Labels: json.Labels });
        return out(201, { Id: id });
      }
      if (req.method === "POST" && /^\/containers\/\w+\/start$/.test(p)) return out(204);
      if (req.method === "GET" && /^\/containers\/\w+\/json$/.test(p)) return out(200, { State: { Running: true } });
      if (req.method === "DELETE" && (m = p.match(/^\/containers\/(\w+)$/))) {
        if (m[1] === "gone") return out(404, { message: "no such container" });
        containers = containers.filter((c) => c.Id !== m![1]);
        return out(204);
      }
      if (req.method === "GET" && p === "/containers/json") return out(200, containers);
      if (req.method === "POST" && (m = p.match(/^\/networks\/([\w-]+)\/(connect|disconnect)$/))) {
        if (failConnect) return out(500, { message: failConnect });
        return out(200, "");
      }
      if (req.method === "POST" && /^\/containers\/\w+\/exec$/.test(p)) {
        const id = `e${++execSeq}`;
        execs.set(id, { cmd: json.Cmd });
        return out(201, { Id: id });
      }
      if (req.method === "POST" && (m = p.match(/^\/exec\/(\w+)\/start$/))) {
        const e = execs.get(m[1]!)!;
        e.detach = json.Detach;
        if (json.Detach) return out(200, "");
        const script = e.cmd.join(" ");
        if (/ cat \/missing/.test(` ${script}`)) return out(200, Buffer.concat([frame(2, "cat: /missing: No such file\n")]), { "content-type": "application/vnd.docker.raw-stream" });
        if (script.includes("result.json")) return out(200, frame(1, '{"hands_done":6}'), { "content-type": "application/vnd.docker.raw-stream" });
        return out(200, Buffer.concat([frame(1, "hello "), frame(2, "warn\n"), frame(1, "world")]), { "content-type": "application/vnd.docker.raw-stream" });
      }
      if (req.method === "GET" && (m = p.match(/^\/exec\/(\w+)\/json$/))) {
        const e = execs.get(m[1]!)!;
        const missing = e.cmd.join(" ").includes("/missing");
        return out(200, { Running: !!e.detach && e.cmd.join(" ").includes("sleep-forever"), ExitCode: missing ? 1 : 0 });
      }
      if (req.method === "PUT" && /^\/containers\/\w+\/archive$/.test(p)) return out(200, "");
      out(404, { message: `unhandled ${req.method} ${p}` });
    });
  });
  await new Promise<void>((r) => server.listen(sock, r));
});
after(async () => { await new Promise((r) => server.close(r)); fs.rmSync(dir, { recursive: true, force: true }); });

const spec = (allowOut: string[]): SandboxSpec => ({
  name: "run-job_1", template: "localhost/sharingan-bot:local", cpu: 2, ramMb: 512, diskGb: 5, autoStopMinutes: 30,
  metadata: { sharingan_job: "job_1", user: "u" }, allowOut,
});
const client = (over: Record<string, unknown> = {}) => new LocalPlatinum({
  engineHost: `unix://${sock}`, network: "botnet", egressNetwork: "egress", binds: [], pidsLimit: 100, arenaHost: "arena", ...over,
});
const last = (method: string, re: RegExp) => [...rec].reverse().find((r) => r.method === method && re.test(r.path));

test("demux splits stdout/stderr frames; tarFile makes a valid ustar entry", () => {
  const d = demux(Buffer.concat([frame(1, "a"), frame(2, "b"), frame(1, "c")]));
  assert.equal(d.stdout.toString(), "ac");
  assert.equal(d.stderr.toString(), "b");
  const t = tarFile("bot.tgz", Buffer.from("xyz"), 0o400, 1_700_000_000);
  assert.equal(t.length % 512, 0);
  assert.equal(t.subarray(0, 7).toString().replace(/\0/g, ""), "bot.tgz");
  assert.equal(t.subarray(257, 262).toString(), "ustar");
  assert.equal(parseInt(t.subarray(124, 135).toString(), 8), 3);
  assert.equal(parseInt(t.subarray(100, 107).toString(), 8), 0o400);
  assert.equal(t.subarray(512, 515).toString(), "xyz");
  // checksum: sum of header bytes with the checksum field read as spaces
  const h = Buffer.from(t.subarray(0, 512));
  h.fill(0x20, 148, 156);
  assert.equal([...h].reduce((a, b) => a + b, 0), parseInt(t.subarray(148, 154).toString(), 8));
});

test("worker-mode sandbox (allowOut []) gets NO network; limits and labels are applied", async () => {
  const sbx = await client().createSandbox(spec([]));
  const create = last("POST", /^\/containers\/create$/)!;
  assert.equal(create.query.get("name"), "run-job_1");
  assert.equal(create.json.Image, "localhost/sharingan-bot:local");
  assert.equal(create.json.HostConfig.NetworkMode, "none");
  assert.equal(create.json.NetworkingConfig, undefined);
  assert.equal(create.json.HostConfig.Memory, 512 * 1024 * 1024);
  assert.equal(create.json.HostConfig.NanoCpus, 2e9);
  assert.equal(create.json.HostConfig.PidsLimit, 100);
  assert.deepEqual(create.json.HostConfig.SecurityOpt, ["no-new-privileges"]);
  assert.equal(create.json.Labels["sharingan.sandbox"], "1");
  assert.equal(create.json.Labels["sharingan.meta.sharingan_job"], "job_1");
  assert.ok(!rec.some((r) => r.path.includes("/networks/")), "no network calls");
  await sbx.delete();
  assert.equal(last("DELETE", /^\/containers\/c\d+$/)!.query.get("force"), "true");
});

test("connect-mode sandbox joins the (internal) arena network; pip window adds and removes the egress network", async () => {
  rec.length = 0;
  const sbx = await client().createSandbox(spec(["arena", "pypi.org", "files.pythonhosted.org"]));
  const create = last("POST", /^\/containers\/create$/)!;
  assert.equal(create.json.HostConfig.NetworkMode, "botnet");
  assert.ok(create.json.NetworkingConfig.EndpointsConfig.botnet);
  assert.equal(last("POST", /^\/networks\/egress\/connect$/)!.json.Container.startsWith("c"), true);
  rec.length = 0;
  await sbx.setEgress(["arena"]);
  assert.ok(last("POST", /^\/networks\/egress\/disconnect$/));
  assert.ok(!last("POST", /^\/networks\/botnet\//), "arena network untouched");
  rec.length = 0;
  await sbx.setEgress(["arena"]);                       // idempotent: nothing to change
  assert.equal(rec.length, 0);
  await sbx.setEgress([]);                              // lock down completely
  assert.ok(last("POST", /^\/networks\/botnet\/disconnect$/));
  await sbx.delete();
});

test("network engine hiccups other than 'already/not attached' surface", async () => {
  const sbx = await client().createSandbox(spec(["arena"]));
  failConnect = "boom";
  await assert.rejects(sbx.setEgress([]), /boom/);
  failConnect = "container is not connected to network botnet";   // idempotent no-op wording
  await sbx.setEgress([]);
  failConnect = "";
  await sbx.delete();
});

test("sh demuxes output and prefixes a kill-timeout; readFile returns bytes; missing file errors", async () => {
  const sbx = await client().createSandbox(spec([]));
  const r = await sbx.sh("echo hi", { timeoutMs: 5000 });
  assert.deepEqual([r.exitCode, r.stdout, r.stderr], [0, "hello world", "warn\n"]);
  const ex = [...rec].reverse().find((x) => x.path.endsWith("/exec") && x.json.Cmd.includes("echo hi"))!;
  assert.deepEqual(ex.json.Cmd.slice(0, 4), ["timeout", "-s", "KILL", "5"]);
  assert.equal(new TextDecoder().decode(await sbx.readFile("/work/result.json")), '{"hands_done":6}');
  await assert.rejects(sbx.readFile("/missing"), /No such file/);
  await sbx.delete();
});

test("writeFile PUTs a tar of one file into its directory", async () => {
  const sbx = await client().createSandbox(spec([]));
  await sbx.writeFile("/opt/sharingan/worker.key", Buffer.from("k3y"), { mode: 0o400 });
  const put = last("PUT", /\/archive$/)!;
  assert.equal(put.query.get("path"), "/opt/sharingan");
  assert.equal(put.raw.subarray(0, 10).toString().replace(/\0/g, ""), "worker.key");
  assert.equal(put.raw.subarray(512, 515).toString(), "k3y");
  assert.equal(parseInt(put.raw.subarray(100, 107).toString(), 8), 0o400);
  await sbx.delete();
});

test("startProcess runs detached with pid file, log redirects, env, cwd and a kill timeout", async () => {
  const sbx = await client().createSandbox(spec([]));
  const p = await sbx.startProcess({ cmd: ["sleep-forever", "1"], cwd: "/work", env: { A: "1" }, timeoutMs: 90_000 });
  const ex = [...rec].reverse().find((x) => x.path.endsWith("/exec") && x.json.Cmd.some((c: string) => c.includes("pid")))!;
  assert.equal(ex.json.WorkingDir, "/work");
  assert.deepEqual(ex.json.Env, ["A=1"]);
  const script = ex.json.Cmd[2] as string;
  assert.match(script, /umask 077; mkdir -p -m 0700 \/tmp\/\.sharingan-proc && echo \$\$ > \/tmp\/\.sharingan-proc\/proc-\d+\.pid && exec timeout -s KILL 90 "\$@" > \/tmp\/\.sharingan-proc\/proc-\d+\.out 2> \/tmp\/\.sharingan-proc\/proc-\d+\.err/);
  assert.deepEqual(ex.json.Cmd.slice(3), ["sh", "sleep-forever", "1"]);
  assert.deepEqual(await p.status(), { running: true });
  await sbx.delete();
});

test("listSandboxes maps labels back to metadata; deleteSandbox tolerates 404", async () => {
  containers = [];
  const c = client();
  await c.createSandbox(spec([]));
  const rows: Array<{ id: string; metadata: Record<string, string>; createdAtMs: number }> = [];
  for await (const r of c.listSandboxes()) rows.push(r);
  assert.equal(rows.length, 1);
  assert.equal(rows[0]!.metadata.sharingan_job, "job_1");
  assert.equal(rows[0]!.createdAtMs, 1_700_000_000_000);
  await c.deleteSandbox("gone");                       // 404 is fine
});

test("a missing image is reported clearly and leaves nothing behind", async () => {
  const c = client();
  const orig = c.engine.json.bind(c.engine);
  c.engine.json = (async (method: string, p: string, o: any) => {
    if (p.startsWith("/containers/create")) throw Object.assign(new Error("engine 404: no such image"), { status: 404, name: "EngineError" });
    return orig(method, p, o);
  }) as typeof c.engine.json;
  await assert.rejects(c.createSandbox(spec([])), /no such image/);
});
