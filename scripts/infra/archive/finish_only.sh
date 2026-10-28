#!/usr/bin/env bash
# Mac-side, detached: wait for an archive job to finish uploading + hashing, then complete + verify it. Never deletes anything.
#   usage: finish_only.sh BOX JOB        (log: scripts/infra/archive/state/JOB.finish.log)
set -u
BOX="$1"; JOB="$2"; HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../../.." && pwd)"
LOG="$HERE/state/$JOB.finish.log"; log() { echo "$(date -u +%FT%TZ) $*" >> "$LOG"; }
log "watching $BOX/$JOB (complete only)"
for i in $(seq 1 720); do
  st=$(BOX=$BOX "$ROOT/scripts/remote.sh" ssh "tail -1 ~/archive/$JOB/run.log" 2>/dev/null)
  [ "$BOX" = dev ] && st=$("$ROOT/scripts/dev.sh" sh "tail -1 ~/archive/$JOB/run.log" 2>/dev/null)
  echo "$st" | grep -q "sha256 done" && break; sleep 60
done
"$HERE/arch" complete --box "$BOX" --name "$JOB" >> "$LOG" 2>&1 && log "COMPLETE OK" || log "complete/verify FAILED"
