// Cheap pre-flight look inside a bot upload: is it a gzip tar, and are the files the job needs in it? Streams the
// decompression (never buffers the unpacked archive), reads only tar headers, and stops as soon as everything wanted was
// seen or `maxBytes` of tar data went by. When the answer is not cheap to get we say "unknown" and let the sandbox decide.
import { createGunzip } from "node:zlib";

/** The upload is not a gzip tarball (or is corrupt). A 400 for the caller. */
export class ArchiveError extends Error {}

export interface ArchiveScan {
  /** Wanted paths that are definitely not in the archive. Empty when the scan could not finish (see `complete`). */
  missing: string[];
  /** The whole archive was walked. false = stopped early because the answer was already known or it was too big to scan. */
  complete: boolean;
}

const normName = (n: string): string => n.replace(/^(\.\/)+/, "").replace(/\/+$/, "");
const cstr = (b: Buffer, from: number, len: number): string => {
  const s = b.subarray(from, from + len);
  const z = s.indexOf(0);
  return s.subarray(0, z < 0 ? s.length : z).toString("utf8");
};
const octal = (b: Buffer, from: number, len: number): number => {
  if (b[from]! & 0x80) { let v = 0; for (let i = from + 1; i < from + len; i++) v = v * 256 + b[i]!; return v; }   // base-256 (huge files)
  const t = cstr(b, from, len).trim();
  return t === "" ? 0 : /^[0-7]+$/.test(t) ? parseInt(t, 8) : NaN;
};
const checksumOk = (h: Buffer): boolean => {
  let sum = 0;
  for (let i = 0; i < 512; i++) sum += i >= 148 && i < 156 ? 32 : h[i]!;
  return sum === octal(h, 148, 8);
};
const pax = (data: Buffer): string | undefined => {
  let p = 0;
  while (p < data.length) {
    const sp = data.indexOf(32, p);
    if (sp < 0) break;
    const len = Number(data.subarray(p, sp).toString());
    if (!Number.isInteger(len) || len <= 0) break;
    const rec = data.subarray(sp + 1, p + len - 1).toString("utf8");
    if (rec.startsWith("path=")) return rec.slice(5);
    p += len;
  }
  return undefined;
};

export async function scanTgz(tgz: Uint8Array, wanted: string[], opts: { maxBytes?: number } = {}): Promise<ArchiveScan> {
  if (tgz.length < 2 || tgz[0] !== 0x1f || tgz[1] !== 0x8b) throw new ArchiveError("the artifact is not a gzip tarball (.tar.gz)");
  const maxBytes = opts.maxBytes ?? 128 * 1024 * 1024;
  const want = new Set(wanted.map(normName));
  const found = new Set<string>();
  let buf: Buffer = Buffer.alloc(0);
  let skip = 0;
  let collect: { kind: "L" | "x"; size: number; pad: number } | undefined;
  let longName: string | undefined;
  let paxPath: string | undefined;
  let first = true;
  let total = 0;
  let complete = false;     // reached the end-of-archive marker
  let unknown = false;      // gave up: too big or unparseable header after the first

  const done = () => unknown || complete || [...want].every((w) => found.has(w));

  const feed = (chunk: Buffer): void => {
    total += chunk.length;
    if (total > maxBytes) { unknown = true; return; }
    buf = buf.length ? Buffer.concat([buf, chunk]) : chunk;
    for (;;) {
      if (skip > 0) {
        const take = Math.min(skip, buf.length);
        buf = buf.subarray(take); skip -= take;
        if (skip > 0) return;
      }
      if (collect) {
        if (buf.length < collect.size + collect.pad) return;
        const content = buf.subarray(0, collect.size);
        buf = buf.subarray(collect.size + collect.pad);
        if (collect.kind === "L") longName = cstr(content, 0, content.length);
        else paxPath = pax(content);
        collect = undefined;
        continue;
      }
      if (buf.length < 512) return;
      const h = buf.subarray(0, 512);
      buf = buf.subarray(512);
      if (h.every((x) => x === 0)) { complete = true; return; }
      if (!checksumOk(h)) {
        if (first) throw new ArchiveError("the artifact is a gzip file but not a tar archive");
        unknown = true; return;
      }
      first = false;
      const type = String.fromCharCode(h[156] || 48);
      const size = octal(h, 124, 12);
      if (!Number.isFinite(size)) { unknown = true; return; }
      const pad = (512 - (size % 512)) % 512;
      if (type === "L" || type === "x") {
        if (size > 64 * 1024) { unknown = true; return; }
        collect = { kind: type, size, pad };
        continue;
      }
      if (type === "g" || type === "K") { skip = size + pad; continue; }
      let name = cstr(h, 0, 100);
      if (cstr(h, 257, 5) === "ustar") { const prefix = cstr(h, 345, 155); if (prefix) name = `${prefix}/${name}`; }
      name = longName ?? paxPath ?? name;
      longName = paxPath = undefined;
      if (type !== "5") found.add(normName(name));
      if (done()) return;
      skip = size + pad;
    }
  };

  const gz = createGunzip();
  try {
    gz.end(Buffer.from(tgz.buffer, tgz.byteOffset, tgz.byteLength));
    for await (const chunk of gz) { feed(chunk as Buffer); if (done()) break; }
    if (!done()) complete = true;                 // stream ended: we saw everything there is
  } catch (e) {
    if (e instanceof ArchiveError) throw e;
    throw new ArchiveError(`the artifact is not a valid gzip file: ${String((e as Error).message).slice(0, 120)}`);
  } finally {
    gz.destroy();
  }
  const sawAll = complete && !unknown;
  return { missing: sawAll ? [...want].filter((w) => !found.has(w)) : [], complete: sawAll };
}
