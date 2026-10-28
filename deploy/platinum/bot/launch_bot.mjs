// Connect a user-uploaded TypeScript/JavaScript bot to the arena (connect mode; ran in deploy/local with a .ts bot).
// env: BOT_FILE (path to .ts/.js/.mjs), ARENA_URL, ARENA_TOKEN, MAX_HANDS (optional)
// Runs under node 22 with --experimental-strip-types (set via NODE_OPTIONS in the image), so .ts bots load directly
// (type-stripping only: no enums, no namespaces, no path aliases).
import { pathToFileURL } from "node:url";
import { run } from "poker-sharingan";

const mod = await import(pathToFileURL(process.env.BOT_FILE).href);
let bot = mod.bot ?? mod.default;
if (typeof bot === "function") {
  // a Bot subclass, or a plain act(state) function
  try { bot = new bot(); } catch { bot = { act: mod.default }; }
}
if (!bot && typeof mod.act === "function") bot = { act: mod.act };
if (!bot || typeof bot.act !== "function") {
  console.error("sharingan-bot: no `bot` export, default export with act(), or act() function in", process.env.BOT_FILE);
  process.exit(2);
}
const maxHands = Number(process.env.MAX_HANDS || 0) || undefined;
process.on("SIGTERM", () => process.exit(0));
console.log("sharingan-bot: connecting");
await run(bot, process.env.ARENA_URL, process.env.ARENA_TOKEN, { maxHands });
