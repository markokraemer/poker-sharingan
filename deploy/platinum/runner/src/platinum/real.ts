// REAL PlatinumClient on @platinum-dev/sdk (0.9.0). One ephemeral microVM per run.
// Not exercised against a live control plane (no credentials); request shapes are covered by test/real.test.ts with a mock fetch.
import { Platinum, type Sandbox as PtSandbox, type ProcessHandle as PtProcess } from "@platinum-dev/sdk";
import type {
  CreateOpts, ExecResult, PlatinumClient, ProcessHandle, ProcessStatus, Sandbox, SandboxInfo, SandboxSpec, StartProcessOpts,
} from "./types.ts";

export interface RealOpts {
  apiUrl: string;
  token: string;
  /** Transport override (tests). */
  fetch?: typeof fetch;
}

/** The API clamps one-shot exec calls to MAX_EXEC_TIMEOUT_MS (85 s); longer scripts must be managed processes. */
const EXEC_LIMIT_MS = 80_000;

export class RealPlatinum implements PlatinumClient {
  readonly kind = "platinum" as const;
  readonly pt: Platinum;

  constructor(o: RealOpts) {
    this.pt = new Platinum({ url: o.apiUrl, token: o.token, fetch: o.fetch });
  }

  async createSandbox(spec: SandboxSpec, opts: CreateOpts = {}): Promise<Sandbox> {
    const sbx = await this.pt.sandboxes.create(
      {
        name: spec.name,
        template: spec.template,
        type: "ephemeral",
        auto_stop_minutes: spec.autoStopMinutes,   // belt and braces; the runner deletes explicitly
        cpu: spec.cpu,
        ram_mb: spec.ramMb,
        disk_gb: spec.diskGb,
        region: spec.region,
        metadata: spec.metadata,
        // allow wins over deny: hostnames in allowOut stay reachable, everything else is dropped (G14: DNS unverified)
        network: { allowOut: spec.allowOut, denyOut: ({ allTraffic }) => [allTraffic] },
      },
      { waitForRunning: true, waitTimeoutMs: opts.timeoutMs ?? 120_000, signal: opts.signal },
    );
    return new RealSandbox(sbx);
  }

  async *listSandboxes(): AsyncIterable<SandboxInfo> {
    for await (const s of this.pt.sandboxes.iterate({ state: "running" })) {
      const row = s as unknown as { id: string; metadata?: Record<string, unknown>; createdAt?: string; created_at?: string };
      const metadata: Record<string, string> = {};
      for (const [k, v] of Object.entries(row.metadata ?? {})) metadata[k] = String(v);
      yield { id: row.id, metadata, createdAtMs: new Date(row.createdAt ?? row.created_at ?? 0).getTime() };
    }
  }

  async deleteSandbox(id: string): Promise<void> {
    try {
      await this.pt.sandboxes.delete(id);
    } catch (e) {
      if ((e as { status?: number }).status === 404) return;
      throw e;
    }
  }
}

class RealSandbox implements Sandbox {
  private readonly sbx: PtSandbox;
  readonly id: string;
  constructor(sbx: PtSandbox) { this.sbx = sbx; this.id = sbx.id; }

  async sh(script: string, opts: { timeoutMs?: number } = {}): Promise<ExecResult> {
    const t = opts.timeoutMs;
    if (t !== undefined && t > EXEC_LIMIT_MS) {
      const p = await this.sbx.process.start({ cmd: ["sh", "-c", script], timeoutMs: t });
      const rec = await p.wait({ timeoutMs: t + 10_000 });
      const l = await p.logs();
      return { exitCode: rec.exit_code, stdout: l.stdout_text, stderr: l.stderr_text };
    }
    const r = await this.sbx.sh(script, { timeoutMs: t });
    return { exitCode: r.exit_code, stdout: r.stdout, stderr: r.stderr };
  }

  async writeFile(path: string, data: Uint8Array, opts: { mode?: number } = {}): Promise<void> {
    await this.sbx.files.write(path, data);
    if (opts.mode !== undefined) (await this.sbx.exec(["chmod", opts.mode.toString(8), path])).check();
  }

  readFile(path: string): Promise<Uint8Array> { return this.sbx.files.read(path); }

  async startProcess(o: StartProcessOpts): Promise<ProcessHandle> {
    const p = await this.sbx.process.start({ cmd: o.cmd, cwd: o.cwd, env: o.env, timeoutMs: o.timeoutMs });
    return new RealProcess(p);
  }

  async setEgress(allowOut: string[]): Promise<void> {
    await this.sbx.updateNetwork({ allowOut, denyOut: ({ allTraffic }) => [allTraffic] });
  }

  usage(): Promise<Record<string, unknown> | undefined> { return this.sbx.usage(); }
  async delete(): Promise<void> { await this.sbx.delete(); }
}

class RealProcess implements ProcessHandle {
  private readonly p: PtProcess;
  constructor(p: PtProcess) { this.p = p; }
  async status(): Promise<ProcessStatus> {
    const i = await this.p.info();
    return i.running ? { running: true } : { running: false, exitCode: i.exit_code };
  }
  async logs(): Promise<{ stdout: string; stderr: string }> {
    const l = await this.p.logs();
    return { stdout: l.stdout_text, stderr: l.stderr_text };
  }
  async kill(): Promise<void> { await this.p.kill(); }
}
