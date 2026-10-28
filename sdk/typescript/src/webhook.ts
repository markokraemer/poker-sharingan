import { createHmac, timingSafeEqual } from "node:crypto";
import { createServer, type Server } from "node:http";
import type { BotLike } from "./bot.js";
import { legalize, safeAction } from "./helpers.js";
import type { YourTurn } from "./types.js";

export const sign = (secret: string, body: string | Buffer): string =>
  "sha256=" + createHmac("sha256", secret).update(body).digest("hex");

export function verify(secret: string, body: string | Buffer, header?: string | null): boolean {
  if (!header) return false;
  const a = Buffer.from(sign(secret, body)), b = Buffer.from(header);
  return a.length === b.length && timingSafeEqual(a, b);
}

/** Pure handler: (status, JSON body) for a POSTed envelope. Works with any HTTP framework. */
export async function handleWebhook(bot: BotLike, body: string, signature?: string | null, secret?: string): Promise<[number, object]> {
  if (secret !== undefined && !verify(secret, body, signature)) return [401, { error: { code: "unauthorized", message: "bad signature" } }];
  const env = JSON.parse(body);
  if (env.type !== "your_turn") return [200, {}];
  const st: YourTurn = { ...env.data, hand_id: env.data.hand_id ?? env.hand_id ?? "", seq: env.data.seq ?? env.seq ?? 0, table_id: env.table_id ?? "", t_ms: env.t_ms ?? 0 };
  let a;
  try { a = await bot.act(st); a = !a ? safeAction(st) : bot.strict ? a : legalize(st, a); } catch { a = safeAction(st); }
  const out: Record<string, unknown> = { hand_id: st.hand_id, decision_id: st.decision_id, type: a.type };
  if (a.to !== undefined && a.type !== "fold" && a.type !== "check") out.to = a.to;
  return [200, out];
}

/** Serve `POST /act` (your_turn -> act) and swallow `POST /events`. Node only. */
export function serveWebhook(bot: BotLike, opts: { port?: number; secret?: string; path?: string } = {}): Promise<Server> {
  const path = opts.path ?? "/act";
  const srv = createServer((req, res) => {
    const chunks: Buffer[] = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", async () => {
      const body = Buffer.concat(chunks).toString();
      const [status, out] = req.url?.replace(/\/+$/, "") === path
        ? await handleWebhook(bot, body, req.headers["x-signature"] as string | undefined, opts.secret)
        : [200, {}] as [number, object];
      res.writeHead(status, { "Content-Type": "application/json" }).end(JSON.stringify(out));
    });
  });
  return new Promise((resolve) => srv.listen(opts.port ?? 8000, () => resolve(srv)));
}
