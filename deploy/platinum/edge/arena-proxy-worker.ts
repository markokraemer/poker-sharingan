// Cloudflare Worker: serve arena.sharingan.poker from the Platinum app URL (or the big-RAM box). Not deployed or tested. INTERIM ONLY.
// Platinum has no custom-domain support (only <slug>.apps.<domain> and <port>-<id>.sbx.<domain>; see DESIGN.md G3),
// so we terminate arena.sharingan.poker on Cloudflare (the repo already deploys site/ with wrangler) and forward.
// WebSocket upgrades pass through fetch() unchanged. Set UPSTREAM to the app URL, e.g. https://sharingan-arena.apps.platinum.dev
export interface Env { UPSTREAM: string }

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    const url = new URL(req.url);
    const target = new URL(url.pathname + url.search, env.UPSTREAM);
    const headers = new Headers(req.headers);
    headers.set("x-forwarded-host", url.host);          // arena ignores it today; useful in logs
    headers.delete("host");
    return fetch(new Request(target, { method: req.method, headers, body: req.body, redirect: "manual" }));
  },
};
