// Runner configuration. All from env; nothing here is secret-bearing except the three tokens. UNTESTED.

function req(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`missing env ${name}`);
  return v;
}
const num = (name: string, d: number) => Number(process.env[name] ?? d);

export const config = {
  // Platinum. A dedicated org + API key scoped `sandboxes:write` (+ `templates:write` only for provision.ts).
  ptApiUrl: process.env.PT_API_URL ?? "https://api.platinum.dev",
  ptToken: req("PT_TOKEN"),
  region: process.env.PT_REGION,                       // keep bots in the arena's region (same-region RTT)
  botTemplate: process.env.BOT_TEMPLATE ?? "sharingan-bot-0-1-0",

  // Arena
  arenaUrl: req("ARENA_URL"),                          // https://arena.sharingan.poker
  arenaAdminToken: process.env.ARENA_ADMIN_TOKEN,      // only needed to register bots when ARENA_OPEN_REGISTRATION=false

  // This service
  port: num("PORT", 8090),
  authToken: req("RUNNER_AUTH_TOKEN"),                 // shared secret with our billing/web backend
  maxConcurrentRuns: num("MAX_CONCURRENT_RUNS", 8),    // keep below the Platinum org's `inflight`/total caps (config.ts tiers)

  // Sandbox shape and limits
  cpu: num("BOT_CPU", 1),
  ramMb: num("BOT_RAM_MB", 1024),
  diskGb: num("BOT_DISK_GB", 5),
  maxArtifactBytes: num("MAX_ARTIFACT_BYTES", 8 * 1024 * 1024),   // sandbox.files.write is capped at 16 MiB
  runDeadlineS: num("RUN_DEADLINE_S", 3 * 3600),       // hard wall clock per job, then abort + delete
  connectTimeoutS: num("CONNECT_TIMEOUT_S", 60),
  pollIntervalMs: num("POLL_INTERVAL_MS", 2000),

  // Pricing. Platinum's real rates live in its dashboard/billing config; set these to match (USD).
  usdPerVcpuSecond: num("USD_PER_VCPU_SECOND", 0.0000125),
  usdPerGibSecond: num("USD_PER_GIB_SECOND", 0.0000030),
  markup: num("MARKUP", 3),                            // multiplier over raw sandbox cost, covers arena/bigmem host + margin
  usdPerHandArenaShare: num("USD_PER_HAND_ARENA_SHARE", 0.0000025), // amortised arena/bigmem cost per hand played
} as const;
