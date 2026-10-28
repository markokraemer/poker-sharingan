#!/usr/bin/env bash
# Runs ON A BOX (started by archive.py start). Uploads every planned 1 GiB byte range with its presigned
# URL, records "key<TAB>part<TAB>etag" in done.tsv (resumable: done parts are skipped), then writes
# sha256 of each source file. No credentials on the box: the URLs are per-part and expire in 7 days.
#   usage: upload_parts.sh plan.tsv [parallel_jobs]
set -u
PLAN="$1"; J="${2:-6}"
TMP="$PWD/tmp"; mkdir -p "$TMP"; touch done.tsv
one() {
  IFS=$'\t' read -r key part off len src url <<< "$1"
  grep -q -P "^\Q$key\E\t$part\t" done.tsv 2>/dev/null && return 0
  t="$TMP/$(echo "$key" | tr '/' '_').$part"
  for try in 1 2 3 4 5; do
    nice -n 10 ionice -c3 dd if="$src" iflag=skip_bytes,count_bytes skip="$off" count="$len" bs=16M status=none of="$t" &&
    [ "$(stat -c %s "$t")" = "$len" ] &&
    etag=$(curl -sS -f --retry 3 -T "$t" -D - -o /dev/null "$url" | tr -d '\r' | awk -F': ' 'tolower($1)=="etag"{print $2}') &&
    [ -n "$etag" ] && { printf '%s\t%s\t%s\n' "$key" "$part" "$etag" >> done.tsv; rm -f "$t"; return 0; }
    echo "$(date -u +%T) retry $try $key part $part" >&2; sleep $((try * 10))
  done
  rm -f "$t"; echo "$(date -u +%T) FAILED $key part $part" >&2; return 1
}
export -f one; export TMP
echo "$(date -u +%T) start: $(wc -l < "$PLAN") parts, $J parallel"
grep -v '^$' "$PLAN" | xargs -d '\n' -P "$J" -I{} bash -c 'one "$@"' _ {}
echo "$(date -u +%T) uploads done: $(wc -l < done.tsv) parts"
cut -f5 "$PLAN" | sort -u | while read -r src; do nice -n 10 ionice -c3 sha256sum "$src"; done > sha256.txt
echo "$(date -u +%T) sha256 done"; rm -rf "$TMP"
