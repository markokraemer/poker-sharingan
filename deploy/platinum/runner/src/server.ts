// Runner service entry point: wires config -> storage -> sandbox backend -> Service -> HTTP.
//   SANDBOX_BACKEND=platinum (default)  real Platinum via @platinum-dev/sdk
//   SANDBOX_BACKEND=local               containers through a Docker-compatible engine socket (deploy/local)
import { ArenaClient } from "./arena.ts";
import { createApp } from "./app.ts";
import { loadConfig } from "./config.ts";
import { createPool, migrate } from "./db.ts";
import { MemoryLedger, PgLedger, type Ledger } from "./ledger.ts";
import { stdoutLog } from "./log.ts";
import { LocalPlatinum } from "./platinum/local.ts";
import type { PlatinumClient } from "./platinum/types.ts";
import { Service } from "./service.ts";
import { MemoryJobStore, PgJobStore, type JobStore } from "./store.ts";
import { makeGateway } from "./stripe.ts";

const cfg = loadConfig();
const log = stdoutLog;

let ledger: Ledger;
let store: JobStore;
let ready: () => Promise<void> = async () => {};
if (cfg.storage === "postgres") {
  const pool = createPool(cfg.databaseUrl!);
  // the database may still be starting (compose): retry for a while
  for (let i = 0; ; i++) {
    try { await migrate(pool); break; } catch (e) {
      if (i >= 30) throw e;
      log("db_wait", { attempt: i + 1, error: String((e as Error).message) });
      await new Promise((r) => setTimeout(r, 1000));
    }
  }
  ledger = new PgLedger(pool);
  store = new PgJobStore(pool);
  ready = async () => { await pool.query("SELECT 1"); };
} else {
  log("memory_storage", { warning: "credits and jobs are lost on restart" });
  ledger = new MemoryLedger();
  store = new MemoryJobStore();
}

let platinum: PlatinumClient;
if (cfg.backend === "local") {
  platinum = new LocalPlatinum({
    engineHost: cfg.local.engineHost, network: cfg.local.network, egressNetwork: cfg.local.egressNetwork,
    binds: cfg.local.binds, pidsLimit: cfg.local.pidsLimit, arenaHost: new URL(cfg.arena.botUrl).hostname,
  });
} else {
  const { RealPlatinum } = await import("./platinum/real.ts");   // the SDK is only loaded when it is used
  platinum = new RealPlatinum({ apiUrl: cfg.platinum.apiUrl, token: cfg.platinum.token! });
}

const arena = new ArenaClient(cfg.arena);
const svc = new Service({ cfg, ledger, store, platinum, arena, gateway: makeGateway(cfg.stripe), log });
await svc.recover();
svc.start();

const server = createApp(svc, cfg, { ready, log, arena });
server.listen(cfg.port, cfg.host, () => log("listening", {
  port: cfg.port, backend: cfg.backend, storage: cfg.storage, arena: cfg.arena.url, billing: cfg.billing.mode, price_usd_per_1k_hands: cfg.pricing.usdPerKHands,
  stripe: cfg.stripe.secretKey ? "live-api" : "simulated-checkout",
}));

let stopping = false;
const shutdown = async (sig: string) => {
  if (stopping) return;
  stopping = true;
  log("shutdown", { signal: sig, in_flight: svc.inFlight() });
  server.close();
  // Give running jobs a bounded time to finish and settle; anything left is failed + refunded by recover() on the next boot.
  await Promise.race([svc.drain(), new Promise((r) => setTimeout(r, Number(process.env.SHUTDOWN_GRACE_MS ?? 20_000)))]);
  svc.stop();
  process.exit(0);
};
process.on("SIGTERM", () => void shutdown("SIGTERM"));
process.on("SIGINT", () => void shutdown("SIGINT"));
