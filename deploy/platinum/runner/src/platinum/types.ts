// The sandbox backend the runner needs, nothing more. Two implementations:
//   real.ts   @platinum-dev/sdk against a Platinum control plane (one microVM per run)
//   local.ts  a Docker-Engine-API container per run (podman or docker) on the arena's network, for dev / CI / the e2e
// Everything above this seam (job.ts, service.ts) is identical in both.

export interface SandboxSpec {
  /** Unique, human readable (used as the container / sandbox name). */
  name: string;
  /** Platinum template name, or (local) an image reference. */
  template: string;
  cpu: number;
  ramMb: number;
  diskGb: number;
  region?: string;
  autoStopMinutes: number;
  /** String metadata; `sharingan_job` is what the orphan reaper keys on. */
  metadata: Record<string, string>;
  /** Hostnames the sandbox may reach; everything else is denied. */
  allowOut: string[];
}

export interface ExecResult { exitCode: number; stdout: string; stderr: string }

export interface ProcessStatus { running: boolean; exitCode?: number }

export interface ProcessHandle {
  status(): Promise<ProcessStatus>;
  logs(): Promise<{ stdout: string; stderr: string }>;
  kill(): Promise<void>;
}

export interface StartProcessOpts {
  cmd: string[];
  cwd?: string;
  env?: Record<string, string>;
  /** Guest-side hard stop for the process. */
  timeoutMs?: number;
}

export interface Sandbox {
  readonly id: string;
  /** Run a shell script to completion (as root). Throws on transport errors, returns non-zero exit codes. */
  sh(script: string, opts?: { timeoutMs?: number }): Promise<ExecResult>;
  writeFile(path: string, data: Uint8Array, opts?: { mode?: number }): Promise<void>;
  readFile(path: string): Promise<Uint8Array>;
  startProcess(opts: StartProcessOpts): Promise<ProcessHandle>;
  /** Replace the egress allowlist (hostnames). Used to drop PyPI after `pip install`. */
  setEgress(allowOut: string[]): Promise<void>;
  /** Backend's own bill for this sandbox so far, if it can say (Platinum: GET .../usage). */
  usage(): Promise<Record<string, unknown> | undefined>;
  delete(): Promise<void>;
}

export interface SandboxInfo { id: string; metadata: Record<string, string>; createdAtMs: number }

export interface CreateOpts { signal?: AbortSignal; timeoutMs?: number }

export interface PlatinumClient {
  readonly kind: "platinum" | "local";
  createSandbox(spec: SandboxSpec, opts?: CreateOpts): Promise<Sandbox>;
  /** Running sandboxes this client can see (for the orphan reaper). */
  listSandboxes(): AsyncIterable<SandboxInfo>;
  /** Delete by id without a handle (reaper). Already-gone is not an error. */
  deleteSandbox(id: string): Promise<void>;
}

export const sleep = (ms: number, signal?: AbortSignal): Promise<void> =>
  new Promise((resolve, reject) => {
    const t = setTimeout(resolve, ms);
    signal?.addEventListener("abort", () => { clearTimeout(t); reject(new Error("aborted")); }, { once: true });
  });
