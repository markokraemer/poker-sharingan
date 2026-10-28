// Runner configuration. Everything comes from env; `loadConfig(env)` is pure so tests pass their own.
//
// Required in every mode:  RUNNER_AUTH_TOKEN, ARENA_URL, ARENA_RUNNER_TOKEN, STRIPE_WEBHOOK_SECRET
// Required with Postgres:  RUNNER_DATABASE_URL (or DATABASE_URL)         (RUNNER_STORAGE=memory for throwaway runs)
// Required on Platinum:    PT_TOKEN                                      (SANDBOX_BACKEND=platinum, the default)
// Required locally:        SANDBOX_BACKEND=local, LOCAL_BOT_IMAGE, LOCAL_BOT_NETWORK, a container engine socket

export type Env = Record<string, string | undefined>;
export type JobMode = "worker" | "connect";

export interface CreditPack { id: string; priceUsd: number; creditUsd: number; label: string }

export interface Config {
  backend: "platinum" | "local";
  host: string;
  port: number;
  authToken: string;
  /**
   * BILLING_MODE=paid (default): credits are reserved/settled in the ledger, Stripe tops them up.
   * BILLING_MODE=free: jobs cost nothing; per-player quotas apply; public players authenticate with their ARENA bot token
   * (validated against GET /v1/me on the arena). RUNNER_AUTH_TOKEN stays the trusted service token.
   */
  billing: { mode: "paid" | "free"; freeMaxHandsPerDay: number; freeMaxConcurrent: number; authCacheS: number };
  storage: "postgres" | "memory";
  databaseUrl?: string;

  arena: {
    url: string;                 // runner -> arena
    botUrl: string;              // what a sandboxed bot connects to (connect mode); defaults to `url`
    runnerToken: string;         // ARENA_RUNNER_TOKEN: may start/attest hosted runs and POST worker results
    adminToken?: string;         // only if the arena has ARENA_OPEN_REGISTRATION=false
  };

  platinum: { apiUrl: string; token?: string; region?: string; botTemplate: string };

  local: {
    engineHost: string;          // unix:///path/to/docker-compatible.sock | tcp://host:port
    botImage: string;
    network: string;             // sandbox network (should be `internal` so it can reach only the arena)
    egressNetwork?: string;      // joined temporarily for `pip install`
    binds: string[];             // extra bind mounts, "host:container[:ro]" (e.g. the test blueprint)
    pidsLimit: number;
  };

  sandbox: { cpu: number; ramMb: number; diskGb: number; autoStopMinutes: number };

  worker: {
    defaultMode: JobMode;
    /** Shell command run inside the sandbox in worker mode. Placeholders are shell-quoted by the runner. */
    cmdTemplate: string;
    /** Per language: the `--bot` command the worker spawns (the user's entry file path is appended). */
    pipeCmd: { python: string; typescript?: string };
    env: Record<string, string>; // extra env for the worker process (ARENA_BLUEPRINT=..., etc.)
    keyId?: string;              // id of the arena's ARENA_WORKER_KEYS entry (`id=secret`)
    signKey?: string;            // the secret; written (as `id=secret`) to a root-only --sign-key-file inside the sandbox for the run
    resultPath: string;
  };

  limits: {
    maxArtifactBytes: number;
    runDeadlineS: number;
    connectTimeoutS: number;
    pollIntervalMs: number;
    pollFailureLimit: number;
    maxConcurrentRuns: number;
    minHands: number;
    maxHands: number;
    defaultHands: number;
    defaultLanes: number;
    maxLanes: number;
  };

  pricing: {
    usdPerKHands: number;        // THE price: credits charged per 1,000 hands played
    minChargeUsd: number;        // floor for a job that played at least one hand
    failedBotChargeUsd: number;  // flat charge when the user's bot made the run fail before any result (0 = free)
    // COGS model, only used to record cost/margin per job (not billed):
    usdPerVcpuSecond: number;
    usdPerGibSecond: number;
    usdPerHandArenaShare: number;
  };

  stripe: {
    webhookSecret: string;
    secretKey?: string;          // unset = simulated checkout (no network)
    apiBase: string;
    publicBaseUrl: string;       // where Checkout returns the player
    toleranceS: number;
    packs: CreditPack[];
    minTopupUsd: number;
    maxTopupUsd: number;
  };
}

function bad(msg: string): never { throw new Error(`config: ${msg}`); }

export function loadConfig(env: Env = process.env): Config {
  const str = (k: string, d?: string): string | undefined => (env[k] !== undefined && env[k] !== "" ? env[k] : d);
  const req = (k: string): string => str(k) ?? bad(`missing env ${k}`);
  const num = (k: string, d: number): number => {
    const raw = str(k);
    if (raw === undefined) return d;
    const n = Number(raw);
    if (!Number.isFinite(n)) bad(`${k} must be a number, got ${raw}`);
    return n;
  };
  const json = <T>(k: string, d: T): T => {
    const raw = str(k);
    if (raw === undefined) return d;
    try { return JSON.parse(raw) as T; } catch { return bad(`${k} must be JSON`); }
  };

  const backend = (str("SANDBOX_BACKEND", "platinum") as string);
  if (backend !== "platinum" && backend !== "local") bad("SANDBOX_BACKEND must be platinum|local");
  const storage = (str("RUNNER_STORAGE", "postgres") as string);
  if (storage !== "postgres" && storage !== "memory") bad("RUNNER_STORAGE must be postgres|memory");
  const databaseUrl = str("RUNNER_DATABASE_URL") ?? str("DATABASE_URL");
  if (storage === "postgres" && !databaseUrl) bad("missing env RUNNER_DATABASE_URL (or RUNNER_STORAGE=memory)");
  if (backend === "platinum" && !str("PT_TOKEN")) bad("missing env PT_TOKEN (or SANDBOX_BACKEND=local)");
  const defaultMode = (str("DEFAULT_JOB_MODE", "worker") as string);
  if (defaultMode !== "worker" && defaultMode !== "connect") bad("DEFAULT_JOB_MODE must be worker|connect");

  const billingMode = str("BILLING_MODE", "paid") as string;
  if (billingMode !== "paid" && billingMode !== "free") bad("BILLING_MODE must be paid|free");
  const arenaUrl = req("ARENA_URL");
  const packs = json<Array<{ id: string; price_usd: number; credit_usd: number; label?: string }>>("CREDIT_PACKS", [
    { id: "starter", price_usd: 10, credit_usd: 10, label: "$10" },
    { id: "plus", price_usd: 50, credit_usd: 55, label: "$50 (+10%)" },
    { id: "pro", price_usd: 200, credit_usd: 240, label: "$200 (+20%)" },
  ]).map((p) => ({ id: p.id, priceUsd: p.price_usd, creditUsd: p.credit_usd, label: p.label ?? p.id }));

  return {
    backend: backend as Config["backend"],
    host: str("HOST", "0.0.0.0") as string,
    port: num("PORT", 8090),
    authToken: req("RUNNER_AUTH_TOKEN"),
    billing: {
      mode: billingMode,
      freeMaxHandsPerDay: num("FREE_MAX_HANDS_PER_DAY", 1_000_000),
      freeMaxConcurrent: num("FREE_MAX_CONCURRENT_JOBS", 2),
      authCacheS: num("ARENA_AUTH_CACHE_S", 60),
    },
    storage: storage as Config["storage"],
    databaseUrl,

    arena: {
      url: arenaUrl,
      botUrl: str("ARENA_BOT_URL", arenaUrl) as string,
      runnerToken: req("ARENA_RUNNER_TOKEN"),
      adminToken: str("ARENA_ADMIN_TOKEN"),
    },

    platinum: {
      apiUrl: str("PT_API_URL", "https://api.platinum.dev") as string,
      token: str("PT_TOKEN"),
      region: str("PT_REGION"),
      botTemplate: str("BOT_TEMPLATE", "sharingan-bot-0-1-0") as string,
    },

    local: {
      engineHost: str("CONTAINER_HOST", "unix:///var/run/engine.sock") as string,
      botImage: str("LOCAL_BOT_IMAGE", str("BOT_TEMPLATE", "localhost/sharingan-bot:local")) as string,
      network: str("LOCAL_BOT_NETWORK", "sharingan-botnet") as string,
      egressNetwork: str("LOCAL_EGRESS_NETWORK"),
      binds: (str("LOCAL_SANDBOX_BINDS", "") as string).split(",").map((s) => s.trim()).filter(Boolean),
      pidsLimit: num("LOCAL_PIDS_LIMIT", 512),
    },

    sandbox: {
      cpu: num("BOT_CPU", 2),
      ramMb: num("BOT_RAM_MB", 2048),
      diskGb: num("BOT_DISK_GB", 5),
      autoStopMinutes: num("BOT_AUTO_STOP_MINUTES", 30),
    },

    worker: {
      defaultMode: defaultMode as JobMode,
      cmdTemplate: str(
        "WORKER_CMD_TEMPLATE",
        "sharingan-arena run --bot {bot} --track {track} --hands {hands} --lanes {lanes} --out {out} --sign-key-file {keyfile} --player-id {player_id}",
      ) as string,
      pipeCmd: {
        python: str("PIPE_CMD_PYTHON", "sharingan pipe") as string,
        typescript: str("PIPE_CMD_TYPESCRIPT"),
      },
      env: json<Record<string, string>>("WORKER_ENV_JSON", {}),
      keyId: str("WORKER_KEY_ID"),
      signKey: str("WORKER_SIGN_KEY"),
      resultPath: str("WORKER_RESULT_PATH", "/work/result.json") as string,
    },

    limits: {
      maxArtifactBytes: num("MAX_ARTIFACT_BYTES", 8 * 1024 * 1024),
      runDeadlineS: num("RUN_DEADLINE_S", 3 * 3600),
      connectTimeoutS: num("CONNECT_TIMEOUT_S", 60),
      pollIntervalMs: num("POLL_INTERVAL_MS", 2000),
      pollFailureLimit: num("POLL_FAILURE_LIMIT", 8),
      maxConcurrentRuns: num("MAX_CONCURRENT_RUNS", 8),
      minHands: num("MIN_HANDS", 600),
      maxHands: num("MAX_HANDS", 200_000),
      defaultHands: num("DEFAULT_HANDS", 20_000),
      defaultLanes: num("DEFAULT_LANES", 1),
      maxLanes: num("MAX_LANES", 8),
    },

    pricing: {
      usdPerKHands: num("PRICE_USD_PER_1K_HANDS", 0.05),
      minChargeUsd: num("MIN_JOB_CHARGE_USD", 0),
      failedBotChargeUsd: num("FAILED_BOT_CHARGE_USD", 0),
      usdPerVcpuSecond: num("USD_PER_VCPU_SECOND", 0.0000125),
      usdPerGibSecond: num("USD_PER_GIB_SECOND", 0.000003),
      usdPerHandArenaShare: num("USD_PER_HAND_ARENA_SHARE", 0.0000025),
    },

    stripe: {
      webhookSecret: billingMode === "free" ? (str("STRIPE_WEBHOOK_SECRET") ?? "") : req("STRIPE_WEBHOOK_SECRET"),
      secretKey: str("STRIPE_SECRET_KEY"),
      apiBase: str("STRIPE_API_BASE", "https://api.stripe.com") as string,
      publicBaseUrl: str("PUBLIC_BASE_URL", "http://localhost:3000") as string,
      toleranceS: num("STRIPE_TOLERANCE_S", 300),
      packs,
      minTopupUsd: num("MIN_TOPUP_USD", 5),
      maxTopupUsd: num("MAX_TOPUP_USD", 1000),
    },
  };
}
