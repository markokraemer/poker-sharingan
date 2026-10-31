// LOCAL PlatinumClient: each "sandbox" is a container created through the Docker Engine API (rootless podman or docker)
// on the same network as the arena. Same job flow as production, minus the microVM boundary.
//
// Egress model, the local analogue of Platinum's `allowOut:[hosts], denyOut:[0.0.0.0/0]`:
//   allowOut = []                 -> no network at all (worker mode: the runner talks to the arena, the sandbox never does)
//   allowOut includes the arena   -> `network` (compose: an `internal: true` network holding the arena and nothing that
//                                    reaches the internet)
//   allowOut includes other hosts -> `egressNetwork` (a normal network), only while `pip install` runs
// `setEgress` re-computes the set and connects / disconnects the difference, like `updateNetwork` does on Platinum.
import { Engine, EngineError, demux, tarFile } from "./engine.ts";
import type {
  CreateOpts, ExecResult, PlatinumClient, ProcessHandle, ProcessStatus, Sandbox, SandboxInfo, SandboxSpec, StartProcessOpts,
} from "./types.ts";

export interface LocalOpts {
  engineHost: string;
  network: string;
  egressNetwork?: string;
  binds: string[];
  pidsLimit: number;
  /** AppArmor profile for the sandboxes (LOCAL_SANDBOX_APPARMOR); unset = podman's default. */
  apparmor?: string;
  /** The arena's hostname: the one destination that needs no extra network. */
  arenaHost: string;
  engine?: Engine;
}

const LABEL = "sharingan.sandbox";
const META = "sharingan.meta.";

/** Where process pid/stdout/stderr files live inside a sandbox: on a root-only tmpfs no bot can fill or read. */
export const PROC_DIR = "/opt/sharingan/run/proc";

/** tmpfs mounts of every local sandbox (see createSandbox). */
export const LOCAL_TMPFS: Record<string, string> = {
  "/tmp": "rw,nosuid,nodev,size=2g,mode=1777",
  "/var/tmp": "rw,nosuid,nodev,size=256m,mode=1777",
  "/opt/sharingan/bot": "rw,nosuid,nodev,size=1g,mode=0755",
  "/opt/sharingan/deps": "rw,nosuid,nodev,size=2g,mode=0755",
  "/opt/sharingan/run": "rw,nosuid,nodev,size=64m,mode=0700",   // the runner's process logs: root only
};

export class LocalPlatinum implements PlatinumClient {
  readonly kind = "local" as const;
  readonly engine: Engine;
  private readonly o: LocalOpts;

  constructor(o: LocalOpts) {
    this.o = o;
    this.engine = o.engine ?? new Engine({ host: o.engineHost });
  }

  async createSandbox(spec: SandboxSpec, opts: CreateOpts = {}): Promise<Sandbox> {
    if (opts.signal?.aborted) throw new Error("aborted");
    const labels: Record<string, string> = { [LABEL]: "1" };
    for (const [k, v] of Object.entries(spec.metadata)) labels[META + k] = v;
    const want = this.desiredNetworks(spec.allowOut);
    const first = want[0];
    const body = {
      Image: spec.template,
      Cmd: ["sleep", "infinity"],
      Labels: labels,
      StopTimeout: 1,
      HostConfig: {
        Memory: spec.ramMb * 1024 * 1024,
        NanoCpus: Math.round(spec.cpu * 1e9),
        PidsLimit: this.o.pidsLimit,
        NetworkMode: first ?? "none",
        Binds: this.o.binds.length ? this.o.binds : undefined,
        // apparmor: deploy/production/apparmor/sharingan-sandbox (podman's default denies every signal inside the container under crun)
        SecurityOpt: ["no-new-privileges", ...(this.o.apparmor ? [`apparmor=${this.o.apparmor}`] : [])],
        // Everything a bot (or its install) can write is a size-capped tmpfs, charged to the container's memory limit:
        // nothing reaches the host disk. The image itself has no other writable path for the bot users.
        Tmpfs: LOCAL_TMPFS,
        Init: true,   // reaps orphans, so a killed bot leaves no zombie behind
        // NET_ADMIN in the sandbox's own network namespace only: the worker (root) takes the loopback interface down before
        // a bot runs (no TCP state a bot could leave for the next rotation). Bot users have no capabilities.
        CapAdd: ["NET_ADMIN"],
        Ulimits: [{ Name: "core", Soft: 0, Hard: 0 }],   // no core dumps (they would land on the host's handler)
      },
      NetworkingConfig: first ? { EndpointsConfig: { [first]: {} } } : undefined,
    };
    let id: string;
    try {
      const r = await this.engine.json<{ Id: string }>("POST", `/containers/create?name=${encodeURIComponent(spec.name)}`, { json: body, ok: [201] });
      id = r.Id;
    } catch (e) {
      if (e instanceof EngineError && e.status === 404) throw new Error(`local sandbox image not found: ${spec.template} (${e.message})`);
      throw e;
    }
    const sbx = new LocalSandbox(this, id, new Set(first ? [first] : []));
    try {
      await this.engine.json("POST", `/containers/${id}/start`, { ok: [204, 304] });
      const info = await this.engine.json<{ State?: { Running?: boolean } }>("GET", `/containers/${id}/json`);
      if (!info.State?.Running) throw new Error(`container ${spec.name} is not running after start`);
      await sbx.setEgress(spec.allowOut);
    } catch (e) {
      await this.deleteSandbox(id).catch(() => {});
      throw e;
    }
    return sbx;
  }

  async *listSandboxes(): AsyncIterable<SandboxInfo> {
    const filters = encodeURIComponent(JSON.stringify({ label: [`${LABEL}=1`], status: ["running"] }));
    const rows = await this.engine.json<Array<{ Id: string; Created: number; Labels?: Record<string, string> }>>("GET", `/containers/json?filters=${filters}`);
    for (const r of rows ?? []) {
      const metadata: Record<string, string> = {};
      for (const [k, v] of Object.entries(r.Labels ?? {})) if (k.startsWith(META)) metadata[k.slice(META.length)] = v;
      yield { id: r.Id, metadata, createdAtMs: r.Created * 1000 };
    }
  }

  async deleteSandbox(id: string): Promise<void> {
    try {
      await this.engine.json("DELETE", `/containers/${id}?force=true&v=true`, { ok: [200, 204] });
    } catch (e) {
      if (e instanceof EngineError && e.status === 404) return;
      throw e;
    }
  }

  /** Networks a sandbox with this allowlist should be on (see the egress model above). */
  desiredNetworks(allowOut: string[]): string[] {
    const nets: string[] = [];
    if (allowOut.includes(this.o.arenaHost)) nets.push(this.o.network);
    if (allowOut.some((h) => h !== this.o.arenaHost) && this.o.egressNetwork) nets.push(this.o.egressNetwork);
    return nets;
  }

  async networkOp(op: "connect" | "disconnect", net: string, id: string): Promise<void> {
    const r = await this.engine.request("POST", `/networks/${encodeURIComponent(net)}/${op}`, {
      json: op === "connect" ? { Container: id } : { Container: id, Force: true },
    });
    const text = r.body.toString("utf8");
    // already attached / not attached are the idempotent no-ops
    if (r.status >= 300 && !/already|not connected|not attached|no such|not found|exists/i.test(text)) throw new EngineError(r.status, text.slice(0, 200));
  }
}

let procSeq = 0;

class LocalSandbox implements Sandbox {
  readonly id: string;
  private readonly c: LocalPlatinum;
  private networks: Set<string>;

  constructor(c: LocalPlatinum, id: string, networks: Set<string>) { this.c = c; this.id = id; this.networks = networks; }

  private async execCreate(cmd: string[], o: { env?: Record<string, string>; cwd?: string; user?: string }): Promise<string> {
    const env = o.env ? Object.entries(o.env).map(([k, v]) => `${k}=${v}`) : undefined;
    const r = await this.c.engine.json<{ Id: string }>("POST", `/containers/${this.id}/exec`, {
      json: { AttachStdout: true, AttachStderr: true, Tty: false, Cmd: cmd, Env: env, WorkingDir: o.cwd, User: o.user },
      ok: [201],
    });
    return r.Id;
  }

  /** Run to completion and collect output (the engine streams the multiplexed frames until the process exits). */
  private async execRun(cmd: string[], o: { env?: Record<string, string>; cwd?: string; timeoutMs?: number } = {}): Promise<{ exitCode: number; stdout: Buffer; stderr: Buffer }> {
    const execId = await this.execCreate(cmd, o);
    const r = await this.c.engine.request("POST", `/exec/${execId}/start`, { json: { Detach: false, Tty: false }, timeoutMs: (o.timeoutMs ?? 120_000) + 5_000 });
    if (r.status !== 200) throw new EngineError(r.status, r.body.toString("utf8").slice(0, 300));
    const { stdout, stderr } = demux(r.body);
    const info = await this.c.engine.json<{ ExitCode: number; Running: boolean }>("GET", `/exec/${execId}/json`);
    return { exitCode: info.ExitCode, stdout, stderr };
  }

  async sh(script: string, opts: { timeoutMs?: number } = {}): Promise<ExecResult> {
    const cmd = opts.timeoutMs
      ? ["timeout", "-s", "KILL", String(Math.max(1, Math.ceil(opts.timeoutMs / 1000))), "sh", "-c", script]
      : ["sh", "-c", script];
    const r = await this.execRun(cmd, { timeoutMs: opts.timeoutMs });
    return { exitCode: r.exitCode, stdout: r.stdout.toString("utf8"), stderr: r.stderr.toString("utf8") };
  }

  async writeFile(path: string, data: Uint8Array, opts: { mode?: number } = {}): Promise<void> {
    const i = path.lastIndexOf("/");
    const dir = i <= 0 ? "/" : path.slice(0, i);
    const name = path.slice(i + 1);
    const r = await this.c.engine.request("PUT", `/containers/${this.id}/archive?path=${encodeURIComponent(dir)}`, {
      body: tarFile(name, data, opts.mode ?? 0o644), contentType: "application/x-tar",
    });
    if (r.status !== 200) throw new EngineError(r.status, `write ${path}: ${r.body.toString("utf8").slice(0, 200)}`);
  }

  async readFile(path: string): Promise<Uint8Array> {
    const r = await this.execRun(["cat", path]);
    if (r.exitCode !== 0) throw new Error(`read ${path}: ${r.stderr.toString("utf8").trim() || `exit ${r.exitCode}`}`);
    return new Uint8Array(r.stdout);
  }

  async startProcess(o: StartProcessOpts): Promise<ProcessHandle> {
    const n = ++procSeq;
    const base = `${PROC_DIR}/proc-${n}`;
    const lim = o.timeoutMs ? `timeout -s KILL ${Math.max(1, Math.ceil(o.timeoutMs / 1000))} ` : "";
    // pid file first, then exec so the pid stays the command's; stdout/stderr go to files `logs()` reads back. The
    // directory is root-only (0700): the worker's log carries the bot's prints, and no later bot process may read them.
    const script = `umask 077; mkdir -p -m 0700 ${PROC_DIR} && echo $$ > ${base}.pid && exec ${lim}"$@" > ${base}.out 2> ${base}.err`;   // PROC_DIR: root-only tmpfs
    const execId = await this.execCreate(["sh", "-c", script, "sh", ...o.cmd], { env: o.env, cwd: o.cwd });
    const r = await this.c.engine.request("POST", `/exec/${execId}/start`, { json: { Detach: true, Tty: false } });
    if (r.status >= 300) throw new EngineError(r.status, r.body.toString("utf8").slice(0, 300));
    return new LocalProcess(this.c, this, execId, base);
  }

  async setEgress(allowOut: string[]): Promise<void> {
    const want = new Set(this.c.desiredNetworks(allowOut));
    for (const n of want) if (!this.networks.has(n)) { await this.c.networkOp("connect", n, this.id); this.networks.add(n); }
    for (const n of [...this.networks]) if (!want.has(n)) { await this.c.networkOp("disconnect", n, this.id); this.networks.delete(n); }
  }
  async usage(): Promise<Record<string, unknown> | undefined> { return undefined; }
  async delete(): Promise<void> { await this.c.deleteSandbox(this.id); }
}

class LocalProcess implements ProcessHandle {
  private readonly c: LocalPlatinum;
  private readonly sbx: LocalSandbox;
  private readonly execId: string;
  private readonly base: string;
  constructor(c: LocalPlatinum, sbx: LocalSandbox, execId: string, base: string) {
    this.c = c; this.sbx = sbx; this.execId = execId; this.base = base;
  }

  async status(): Promise<ProcessStatus> {
    const i = await this.c.engine.json<{ Running: boolean; ExitCode: number }>("GET", `/exec/${this.execId}/json`);
    return i.Running ? { running: true } : { running: false, exitCode: i.ExitCode };
  }

  async logs(): Promise<{ stdout: string; stderr: string }> {
    const r = await this.sbx.sh(`tail -c 65536 ${this.base}.out 2>/dev/null; echo '@@SPLIT@@'; tail -c 65536 ${this.base}.err 2>/dev/null`);
    const [stdout = "", stderr = ""] = r.stdout.split("@@SPLIT@@\n");
    return { stdout, stderr };
  }

  async kill(): Promise<void> {
    await this.sbx.sh(`kill -KILL "$(cat ${this.base}.pid 2>/dev/null)" 2>/dev/null; true`);
  }
}
