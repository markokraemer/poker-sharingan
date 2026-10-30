// Pre-flight look inside a bot upload (gzip tar, entry file present) and the log cleaner used for error texts.
import assert from "node:assert/strict";
import { gzipSync } from "node:zlib";
import { describe, test } from "node:test";
import { ArchiveError, scanTgz } from "../src/archive.ts";
import { cleanLog } from "../src/logclean.ts";
import { makeTgz } from "./helpers.ts";

describe("scanTgz", () => {
  test("finds the entry (also ./ prefixed and nested), reports what is missing", async () => {
    const t = makeTgz({ "./bot.py": "x", "sub/dir/req.txt": "y", "data.bin": "z".repeat(3000) });
    assert.deepEqual(await scanTgz(t, ["bot.py", "sub/dir/req.txt"]), { missing: [], complete: false });
    assert.deepEqual((await scanTgz(t, ["bot.py", "nope.py"])).missing, ["nope.py"]);
    assert.deepEqual((await scanTgz(makeTgz({ "other.py": "x" }), ["bot.py"])).missing, ["bot.py"]);
  });
  test("not gzip / not tar / corrupt gzip are ArchiveErrors", async () => {
    await assert.rejects(scanTgz(Buffer.from("fake-tgz"), ["bot.py"]), ArchiveError);
    await assert.rejects(scanTgz(gzipSync(Buffer.from("just some text, 512 bytes of it".padEnd(600, "x"))), ["bot.py"]), /not a tar archive/);
    const noise = Array.from({ length: 4000 }, (_, i) => (i * 2654435761 >>> 0).toString(16)).join("");
    const big = makeTgz({ "a.bin": noise, "b.bin": noise.split("").reverse().join("") });
    await assert.rejects(scanTgz(big.subarray(0, Math.floor(big.length / 2)), ["nope.py"]), ArchiveError);
  });
  test("an archive too big to scan cheaply is 'unknown', not 'missing'", async () => {
    const r = await scanTgz(makeTgz({ "big.bin": "q".repeat(5000), "bot.py": "x" }), ["bot.py"], { maxBytes: 1024 });
    assert.deepEqual(r, { missing: [], complete: false });
  });
});

describe("cleanLog", () => {
  const tb = "Traceback (most recent call last):\n  File \"bot.py\", line 3, in <module>\n    1/0\nZeroDivisionError: division by zero\n";
  test("dedupes identical tracebacks and lines across lanes (lane prefixes ignored), keeps different ones", () => {
    const out = cleanLog(`${tb.split("\n").map((l) => (l ? `[lane 0] ${l}` : l)).join("\n")}${tb}${tb}warn: slow\nwarn: slow\nTraceback (most recent call last):\n  File "bot.py", line 9, in f\nKeyError: 'x'\n`);
    assert.equal(out.split("Traceback").length - 1, 2);
    assert.equal(out.split("ZeroDivisionError").length - 1, 1);
    assert.equal(out.split("warn: slow").length - 1, 1);
    assert.match(out, /KeyError: 'x'/);
  });
  test("drops progress lines, drops the score line only when asked, tail-limits on a line boundary", () => {
    const text = `  ladder-v1 100/600 hands (9 hands/s)\n{"hands_done":1,"hands_total":2}\nboom\nladder-v1: +1.0 \u00b1 2.0 mbb/game (aivat) over 600 hands on ladder-v1; 9 hands/s; signed\n`;
    assert.equal(cleanLog(text, { dropScore: true }), "boom");
    assert.match(cleanLog(text), /mbb\/game/);
    const long = Array.from({ length: 50 }, (_, i) => `line ${i} ${"x".repeat(20)}`).join("\n");
    const cut = cleanLog(long, { maxChars: 100 });
    assert.ok(cut.length <= 100 && cut.startsWith("line ") && cut.endsWith("line 49 " + "x".repeat(20)), cut);
  });
});
