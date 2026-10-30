// Cloudflare Worker `sharingan-arena-edge`: serves https://arena.sharingan.poker by forwarding every request (REST, WebSocket
// /v1/play, runner job API, inference) to the origin, which does the path routing (Caddy: runner / serve / arena).
//
// The origin only accepts traffic that carries the shared secret header, so the Worker is the only way in:
//   wrangler secret put EDGE_SECRET       (same value as EDGE_SECRET in /srv/sharingan/secrets/caddy.env on the host)
//   vars.UPSTREAM                          e.g. https://51-158-203-112.sslip.io
// WebSocket upgrades pass through fetch() unchanged.
export interface Env { UPSTREAM: string; EDGE_SECRET: string }

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    const url = new URL(req.url);
    // The API has no page of its own: send people who open it in a browser to the docs.
    if (url.pathname === "/" && (req.method === "GET" || req.method === "HEAD")) return Response.redirect("https://sharingan.poker/docs/", 302);
    const target = new URL(url.pathname + url.search, env.UPSTREAM);
    const headers = new Headers(req.headers);
    headers.delete("host");
    headers.set("x-edge-secret", env.EDGE_SECRET);
    headers.set("x-forwarded-host", url.host);
    const ip = req.headers.get("cf-connecting-ip");
    if (ip) headers.set("x-client-ip", ip);   // Caddy turns it into X-Forwarded-For (the arena rate-limits anonymous callers per IP)
    return fetch(new Request(target, { method: req.method, headers, body: req.body, redirect: "manual" }));
  },
};
