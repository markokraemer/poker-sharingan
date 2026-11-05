import assert from "node:assert/strict";
import { test } from "node:test";
import worker from "./arena-proxy-worker.ts";
import type { Env } from "./arena-proxy-worker.ts";

const env: Env = { UPSTREAM: "https://origin.example", EDGE_SECRET: "test-only-secret" };

test("public access fails closed before any origin request", async (t) => {
  let calls = 0;
  t.mock.method(globalThis, "fetch", async () => { calls++; return new Response("origin"); });
  for (const flag of [undefined, "false", "TRUE", "1", ""]) {
    for (const path of [
      "/", "/v1/leaderboard", "/v1/leaderboard?public=true",
      "/v1/benchmarks/sharingan-1", "/v1/benchmarks/sharingan-1/runs/w_example",
      "/v1/players/example", "/v1/matches", "/v1/hands/example",
      "/%76%31/leaderboard", "/v1/play", "/v1/act", "/readyz",
    ]) {
      const response = await worker.fetch(new Request("https://arena.example" + path, {
        headers: { authorization: "Bearer test-token", "x-edge-secret": "forged" },
      }), { ...env, PUBLIC_ARENA_ENABLED: flag });
      assert.equal(response.status, 503, path);
      assert.equal(response.headers.get("cache-control"), "no-store");
      assert.equal(response.headers.get("x-robots-tag"), "noindex, nofollow");
      assert.deepEqual(await response.json(), { error: {
        code: "public_arena_unavailable", message: "The public arena is temporarily unavailable.",
      } });
    }
  }
  assert.equal(calls, 0);
});

test("writes and WebSocket handshakes cannot bypass maintenance", async (t) => {
  let calls = 0;
  t.mock.method(globalThis, "fetch", async () => { calls++; return new Response("origin"); });
  for (const method of ["POST", "PUT", "PATCH", "DELETE", "OPTIONS"]) {
    assert.equal((await worker.fetch(new Request("https://arena.example/v1/jobs", { method }), env)).status, 503);
  }
  assert.equal((await worker.fetch(new Request("https://arena.example/v1/play", {
    headers: { upgrade: "websocket", connection: "Upgrade" },
  }), env)).status, 503);
  assert.equal(calls, 0);
});

test("HEAD responses are empty and still fail closed", async () => {
  const response = await worker.fetch(new Request("https://arena.example/v1/leaderboard", { method: "HEAD" }), env);
  assert.equal(response.status, 503);
  assert.equal(await response.text(), "");
});

test("explicit reopening retains forwarding and replaces untrusted proxy headers", async (t) => {
  const seen: Request[] = [];
  t.mock.method(globalThis, "fetch", async (request: Request) => {
    seen.push(request);
    return new Response("operator-enabled-origin");
  });
  const response = await worker.fetch(new Request("https://arena.example/v1/leaderboard?track=sharingan-1", {
    headers: { "x-edge-secret": "forged", "cf-connecting-ip": "192.0.2.4" },
  }), { ...env, PUBLIC_ARENA_ENABLED: "true" });
  assert.equal(await response.text(), "operator-enabled-origin");
  assert.equal(seen.length, 1);
  assert.equal(seen[0].url, "https://origin.example/v1/leaderboard?track=sharingan-1");
  assert.equal(seen[0].headers.get("x-edge-secret"), env.EDGE_SECRET);
  assert.equal(seen[0].headers.get("x-forwarded-host"), "arena.example");
  assert.equal(seen[0].headers.get("x-client-ip"), "192.0.2.4");
});

test("the enabled root redirect does not hit the origin", async (t) => {
  let calls = 0;
  t.mock.method(globalThis, "fetch", async () => { calls++; return new Response("origin"); });
  const response = await worker.fetch(new Request("https://arena.example/"), { ...env, PUBLIC_ARENA_ENABLED: "true" });
  assert.equal(response.status, 302);
  assert.equal(response.headers.get("location"), "https://sharingan.poker/docs/");
  assert.equal(calls, 0);
});
