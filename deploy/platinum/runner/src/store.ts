// Job store. In-memory + JSON snapshot so the skeleton runs standalone. REPLACE with Postgres before launch:
// jobs must survive a runner restart, and the reaper (job.ts: reapOrphans) relies on sandbox metadata, not on this map.
import { readFileSync, writeFileSync, existsSync } from "node:fs";

export type JobStatus =
  | "queued" | "provisioning" | "connecting" | "running" | "finalizing" | "finished" | "failed" | "aborted";

export interface JobInput {
  userId: string;
  botName: string;
  language: "python" | "typescript";
  /** tar.gz of the bot directory, base64. Must contain `entry`. */
  artifactTgzBase64: string;
  entry: string;                    // path inside the archive, e.g. "bot.py"
  requirementsFile?: string;        // python: requirements.txt inside archive (needs PyPI egress during setup)
  track: string;                    // "ladder-v1" | "sharingan-1"
  hands: number;                    // rounded up to a multiple of 6 by the arena
  lanes?: number;
  idempotencyKey: string;           // = credit reservation id from the billing service
}

export interface Job {
  id: string;
  input: Omit<JobInput, "artifactTgzBase64">;
  status: JobStatus;
  error?: string;
  createdAt: number;
  sandboxId?: string;
  arenaPlayerId?: string;
  runId?: string;
  handsDone?: number;
  handsTotal?: number;
  runningMbb?: number | null;
  mbbPerGame?: number | null;
  ci95Mbb?: number | null;
  verified?: boolean;
  sandboxSeconds?: number;
  platinumUsd?: number;
  chargeUsd?: number;
}

const FILE = process.env.JOB_STORE_FILE ?? "./jobs.json";
const jobs = new Map<string, Job>(existsSync(FILE) ? Object.entries(JSON.parse(readFileSync(FILE, "utf8"))) : []);
const persist = () => writeFileSync(FILE, JSON.stringify(Object.fromEntries(jobs)));

export const store = {
  get: (id: string) => jobs.get(id),
  byKey: (key: string) => [...jobs.values()].find((j) => j.input.idempotencyKey === key),
  active: () => [...jobs.values()].filter((j) => !["finished", "failed", "aborted"].includes(j.status)),
  put(j: Job) { jobs.set(j.id, j); persist(); },
  patch(id: string, p: Partial<Job>) { const j = jobs.get(id)!; Object.assign(j, p); persist(); return j; },
};
