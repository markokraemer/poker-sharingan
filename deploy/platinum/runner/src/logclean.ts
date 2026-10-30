// Turn raw worker / bot output into something a person can read: one copy of every traceback (the match worker runs
// several lanes and each prints the same crash), no machine progress chatter and, when asked, no score line.
//
// A worker prints, on stderr, progress ("  ladder-v1 300/600 hands (45 hands/s)") and a closing summary
// ("ladder-v1: +12.5 ± 3.2 mbb/game (aivat) over 600 hands on ladder-v1; 45 hands/s; 600 FORFEIT hands; signed"). On a
// forfeited or rejected run that summary reads like a score, so those runs drop it.

const LANE_PREFIX = /^\s*(?:\[(?:lane\s*)?\d+\]|\(lane \d+\)|lane[ _-]?\d+\s*[:|]) ?/i;
const indented = (l: string) => /^[ \t]/.test(l);
const isProgress = (l: string) => /\d+\s*\/\s*\d+\s*hands/i.test(l) || (l.trimStart().startsWith("{") && /"hands_(?:done|total)"|"hands_planned"/.test(l));
export const isScoreLine = (l: string) => /mbb\/game/i.test(l) && /\bhands\b/i.test(l);

export interface CleanOpts { maxChars?: number; dropScore?: boolean }

/** De-duplicated excerpt (first occurrence wins, order kept), tail-limited to `maxChars` on a line boundary. */
export function cleanLog(text: string, opts: CleanOpts = {}): string {
  const maxChars = opts.maxChars ?? 800;
  const lines = text.replace(/\r/g, "").split("\n").map((l) => l.replace(LANE_PREFIX, "").trimEnd())
    .filter((l) => l.trim() !== "" && !isProgress(l) && !(opts.dropScore && isScoreLine(l)));
  const seen = new Set<string>();
  const out: string[] = [];
  for (let i = 0; i < lines.length; i++) {
    let chunk = [lines[i]!];
    // a traceback / stack is a header plus its indented body; Python's ends with one unindented exception line
    if (!indented(lines[i]!) && indented(lines[i + 1] ?? "")) {
      let j = i + 1;
      while (j < lines.length && indented(lines[j]!)) j++;
      if (/^Traceback/.test(lines[i]!) && j < lines.length) j++;
      chunk = lines.slice(i, j);
      i = j - 1;
    }
    const key = chunk.join("\n");
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(key);
  }
  const s = out.join("\n");
  if (s.length <= maxChars) return s;
  const cut = s.slice(-maxChars);
  const nl = cut.indexOf("\n");
  return nl >= 0 && nl < cut.length - 1 ? cut.slice(nl + 1) : cut;
}
