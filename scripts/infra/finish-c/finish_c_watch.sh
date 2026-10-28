#!/usr/bin/env bash
# Mac-side, detached (nohup + caffeinate -i): waits for box A's ~/finish/DONE (written by box_finish.sh),
# then archives the final C state + compact + eval results to the bucket and deletes the server ONLY after
# both uploads are complete and size-verified. Anything unexpected -> stop without deleting (box keeps billing).
#   usage: finish_c_watch.sh SERVER_ID ZONE TAG        (log: scripts/infra/archive/state/finish-c-TAG.log)
set -u
SID="$1"; ZONE="$2"; TAG="$3"
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../../.." && pwd)"; AR="$ROOT/scripts/infra/archive"
LOG="$AR/state/finish-c-$TAG.log"
log() { echo "$(date -u +%FT%TZ) $*" >> "$LOG"; }
notify() { osascript -e "display notification \"$1\" with title \"Pluribus C\"" >/dev/null 2>&1 || true; }
R() { BOX=A "$ROOT/scripts/remote.sh" ssh "$@"; }
log "watching for ~/finish/DONE on A (server $SID)"
for i in $(seq 1 2880); do   # up to 10 days, 5 min steps
  st=$(R 'ls ~/finish/DONE ~/finish/FAILED 2>/dev/null; cat ~/pluribus/data/run-medium/progress.txt 2>/dev/null' 2>/dev/null)
  echo "$st" | grep -q FAILED && { log "box reported FAILED — NOT deleting. $(R 'tail -3 ~/finish/finish.log')"; notify "C finish FAILED - box still billing"; exit 1; }
  echo "$st" | grep -q DONE && break
  [ $((i % 12)) = 0 ] && log "progress $(echo "$st" | tail -1)"
  sleep 300
done
echo "$st" | grep -q DONE || { log "timeout — NOT deleting"; notify "C finish watcher timed out"; exit 1; }
log "box DONE: $(R 'tail -4 ~/finish/finish.log')"

R "cd ~/finish && tar cf eval-results.tar --exclude='*.plcp' . && cp ~/pluribus/train.log ~/finish/train.log 2>/dev/null; true"
M=/home/ubuntu/pluribus/data/run-medium
"$AR/arch" plan --box A --name "C-final-$TAG" --class STANDARD \
  "/home/ubuntu/finish/C-medium-final.plcp=blueprints/compact/C-medium-final-$TAG.plcp" \
  "/home/ubuntu/finish/eval-results.tar=eval/eval-results-C-final-$TAG.tar" >> "$LOG" 2>&1 || { log "plan (std) failed"; exit 1; }
"$AR/arch" plan --box A --name "C-raw-$TAG" --class GLACIER \
  "$M/blueprint.bin=raw/C-medium-final-$TAG/blueprint.bin" "$M/snapshots.sum=raw/C-medium-final-$TAG/snapshots.sum" \
  "$M/progress.txt=raw/C-medium-final-$TAG/progress.txt" "$M/config.txt=raw/C-medium-final-$TAG/config.txt" \
  "$M/eval.jsonl=raw/C-medium-final-$TAG/eval.jsonl" "/home/ubuntu/finish/train.log=raw/C-medium-final-$TAG/train.log" >> "$LOG" 2>&1 || { log "plan (raw) failed"; exit 1; }
"$AR/arch" start --box A --name "C-final-$TAG" --jobs 4 >> "$LOG" 2>&1
"$AR/arch" start --box A --name "C-raw-$TAG" --jobs 6 >> "$LOG" 2>&1
log "uploads started"

"$AR/finish_only.sh" A "C-final-$TAG"
grep -q "COMPLETE OK" "$AR/state/C-final-$TAG.finish.log" || { log "compact/eval upload did not verify — NOT deleting"; notify "C upload failed - box still billing"; exit 1; }
"$AR/finish_then_delete.sh" A "C-raw-$TAG" "$SID" "$ZONE"
if grep -q "DELETED $SID" "$AR/state/C-raw-$TAG.finish.log"; then log "ALL DONE, server deleted"; notify "C finished, archived, server deleted"
else log "raw archive or delete failed — check $AR/state/C-raw-$TAG.finish.log"; notify "C archive/delete needs attention"; fi
