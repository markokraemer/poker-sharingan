// The REAL PlatinumClient (SDK adapter) driven through the SDK's `fetch` transport override: asserts the requests we would
// send to a Platinum control plane (create body incl. network lockdown, files, processes, egress update, usage, delete).
// No network, no credentials.
import assert from "node:assert/strict";
import { test } from "node:test";
import { RealPlatinum } from "../src/platinum/real.ts";
import type { SandboxSpec } from "../src/platinum/types.ts";

interface Req { method: string; path: string; query: URLSearchParams; body: any; raw: unknown }

function harness(routes: Array<[RegExp, (r: Req) => unknown]>) {
  const seen: Req[] = [];
  const fakeFetch = (async (input: URL | string, init: RequestInit = {}) => {
    const u = new URL(String(input));
    const raw = init.body;
    let body: any;
    if (typeof raw === "string") { try { body = JSON.parse(raw); } catch { body = raw; } }
    const r: Req = { method: String(init.method ?? "GET"), path: u.pathname, query: u.searchParams, body, raw };
    seen.push(r);
    assert.equal((init.headers as Record<string, string>).authorization, "Bearer pt_live_fake");
    for (const [re, h] of routes) {
      if (re.test(`${r.method} ${r.path}`)) {
        const out = h(r);
        return new Response(out === undefined ? "{}" : JSON.stringify(out), { status: 200, headers: { "content-type": "application/json" } });
      }
    }
    return new Response(JSON.stringify({ error: `unrouted ${r.method} ${r.path}` }), { status: 404 });
  }) as unknown as typeof fetch;
  return { seen, client: new RealPlatinum({ apiUrl: "https://api.platinum.test", token: "pt_live_fake", fetch: fakeFetch }) };
}

const spec: SandboxSpec = {
  name: "run-job_1", template: "sharingan-bot-0-1-0", cpu: 2, ramMb: 2048, diskGb: 5, region: "eu-west", autoStopMinutes: 30,
  metadata: { sharingan_job: "job_1", user: "u" }, allowOut: [],
};
const b64 = (s: string) => Buffer.from(s).toString("base64");

test("create: ephemeral sandbox from the template, waits for running, egress denied except the allowlist", async () => {
  const { client, seen } = harness([[/^POST \/v1\/sandboxes$/, () => ({ id: "sbx_1", state: "running", host_id: "h1" })]]);
  const sbx = await client.createSandbox({ ...spec, allowOut: ["arena.sharingan.poker"] });
  assert.equal(sbx.id, "sbx_1");
  const r = seen[0]!;
  assert.equal(r.query.get("wait_for_state"), "running");
  assert.deepEqual(
    { template: r.body.template, type: r.body.type, cpu: r.body.cpu, ram_mb: r.body.ram_mb, disk_gb: r.body.disk_gb, region: r.body.region, name: r.body.name, auto_stop_minutes: r.body.auto_stop_minutes },
    { template: "sharingan-bot-0-1-0", type: "ephemeral", cpu: 2, ram_mb: 2048, disk_gb: 5, region: "eu-west", name: "run-job_1", auto_stop_minutes: 30 },
  );
  assert.deepEqual(r.body.metadata, { sharingan_job: "job_1", user: "u" });
  assert.deepEqual(r.body.network, { allow_out: ["arena.sharingan.poker"], deny_out: ["0.0.0.0/0"] });
});

test("create with no allowed hosts denies all traffic (worker mode)", async () => {
  const { client, seen } = harness([[/^POST \/v1\/sandboxes$/, () => ({ id: "sbx_1", state: "running", host_id: "h1" })]]);
  await client.createSandbox(spec);
  assert.deepEqual(seen[0]!.body.network, { allow_out: [], deny_out: ["0.0.0.0/0"] });
});

test("files, shell, egress update, usage and delete map to the documented endpoints", async () => {
  const { client, seen } = harness([
    [/^POST \/v1\/sandboxes$/, () => ({ id: "sbx_1", state: "running", host_id: "h1" })],
    [/^PUT \/v1\/sandboxes\/sbx_1\/files$/, () => ({})],
    [/^GET \/v1\/sandboxes\/sbx_1\/files$/, () => ({})],
    [/^POST \/v1\/sandboxes\/sbx_1\/exec$/, (r) => ({ result: { stdout: "ok", stderr: "", exit_code: r.body.cmd.includes("false") ? 1 : 0 } })],
    [/^POST \/v1\/sandboxes\/sbx_1\/network$/, () => ({ ok: true })],
    [/^GET \/v1\/sandboxes\/sbx_1\/usage$/, () => ({ cost_usd: 0.0123 })],
    [/^DELETE \/v1\/sandboxes\/sbx_1$/, () => ({})],
  ]);
  const sbx = await client.createSandbox(spec);
  await sbx.writeFile("/tmp/bot.tgz", new Uint8Array([1, 2, 3]));
  const put = seen.find((r) => r.method === "PUT")!;
  assert.equal(put.query.get("path"), "/tmp/bot.tgz");
  assert.deepEqual([...(put.raw as Uint8Array)], [1, 2, 3]);

  await sbx.writeFile("/opt/sharingan/worker.key", new TextEncoder().encode("k"), { mode: 0o400 });
  const chmod = seen.filter((r) => r.path.endsWith("/exec")).pop()!;
  assert.deepEqual(chmod.body.cmd, ["chmod", "400", "/opt/sharingan/worker.key"]);

  const ok = await sbx.sh("echo ok", { timeoutMs: 10_000 });
  assert.deepEqual(ok, { exitCode: 0, stdout: "ok", stderr: "" });
  assert.deepEqual(seen.filter((r) => r.path.endsWith("/exec")).pop()!.body.cmd, ["sh", "-c", "echo ok"]);
  assert.equal((await sbx.sh("false")).exitCode, 1);

  await sbx.setEgress([]);
  assert.deepEqual(seen.find((r) => r.path.endsWith("/network"))!.body, { allow_out: [], deny_out: ["0.0.0.0/0"] });

  assert.deepEqual(await sbx.usage(), { cost_usd: 0.0123 });
  await sbx.delete();
  assert.ok(seen.some((r) => r.method === "DELETE" && r.path === "/v1/sandboxes/sbx_1"));
});

test("long scripts (pip install) run as managed processes, not one-shot exec (API clamps exec to 85 s)", async () => {
  const { client, seen } = harness([
    [/^POST \/v1\/sandboxes$/, () => ({ id: "sbx_1", state: "running", host_id: "h1" })],
    [/^POST \/v1\/sandboxes\/sbx_1\/processes$/, () => ({ handle: "p_abc", pid: 7 })],
    [/^GET \/v1\/sandboxes\/sbx_1\/processes\/p_abc$/, () => ({ handle: "p_abc", pid: 7, cmd: [], running: false, exit_code: 3 })],
    [/^GET \/v1\/sandboxes\/sbx_1\/processes\/p_abc\/logs$/, () => ({ stdout: b64("out"), stderr: b64("boom"), running: false, exit_code: 3 })],
  ]);
  const sbx = await client.createSandbox(spec);
  const r = await sbx.sh("pip install -r r.txt", { timeoutMs: 300_000 });
  assert.deepEqual(r, { exitCode: 3, stdout: "out", stderr: "boom" });
  const start = seen.find((x) => x.path.endsWith("/processes"))!;
  assert.deepEqual(start.body.cmd, ["sh", "-c", "pip install -r r.txt"]);
  assert.equal(start.body.timeout_ms, 300_000);
  assert.ok(!seen.some((x) => x.path.endsWith("/exec")));
});

test("startProcess: argv, cwd, env and guest-side timeout; status and logs map to the process API", async () => {
  const { client, seen } = harness([
    [/^POST \/v1\/sandboxes$/, () => ({ id: "sbx_1", state: "running", host_id: "h1" })],
    [/^POST \/v1\/sandboxes\/sbx_1\/processes$/, () => ({ handle: "p_1", pid: 9 })],
    [/^GET \/v1\/sandboxes\/sbx_1\/processes\/p_1$/, () => ({ handle: "p_1", running: true, exit_code: 0 })],
    [/^GET \/v1\/sandboxes\/sbx_1\/processes\/p_1\/logs$/, () => ({ stdout: b64("a"), stderr: b64("b"), running: true })],
    [/^DELETE \/v1\/sandboxes\/sbx_1\/processes\/p_1$/, () => ({ ok: true, handle: "p_1" })],
  ]);
  const sbx = await client.createSandbox(spec);
  const p = await sbx.startProcess({ cmd: ["sh", "-c", "run"], cwd: "/work", env: { K: "v" }, timeoutMs: 5000 });
  const start = seen.find((x) => x.path.endsWith("/processes"))!;
  assert.deepEqual(start.body, { cmd: ["sh", "-c", "run"], cwd: "/work", env: { K: "v" }, timeout_ms: 5000 });
  assert.deepEqual(await p.status(), { running: true });
  assert.deepEqual(await p.logs(), { stdout: "a", stderr: "b" });
  await p.kill();
  assert.ok(seen.some((x) => x.method === "DELETE" && x.path === "/v1/sandboxes/sbx_1/processes/p_1"));
});

test("listSandboxes pages through running sandboxes and exposes metadata; deleteSandbox ignores 404", async () => {
  const { client } = harness([
    [/^GET \/v1\/sandboxes$/, () => ({ rows: [{ id: "a", metadata: { sharingan_job: "j1" }, createdAt: "2026-01-01T00:00:00Z" }, { id: "b", metadata: {} }], total: 2, has_more: false })],
  ]);
  const out: Array<{ id: string; metadata: Record<string, string> }> = [];
  for await (const s of client.listSandboxes()) out.push(s);
  assert.deepEqual(out.map((s) => s.id), ["a", "b"]);
  assert.equal(out[0]!.metadata.sharingan_job, "j1");
  await client.deleteSandbox("missing");       // unrouted -> 404 -> swallowed
});
