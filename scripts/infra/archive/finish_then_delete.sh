#!/usr/bin/env bash
# Mac-side, detached: wait until an archive job on a box has uploaded every part and written sha256,
# complete + verify it (scripts/infra/archive/arch complete exits non-zero on any size mismatch / missing part),
# and ONLY THEN delete that Scaleway server. Anything unexpected → stop without deleting.
#   usage: finish_then_delete.sh BOX JOB SERVER_ID ZONE        (log: scripts/infra/archive/state/JOB.finish.log)
set -u
BOX="$1"; JOB="$2"; SID="$3"; ZONE="$4"
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../../.." && pwd)"
LOG="$HERE/state/$JOB.finish.log"
log() { echo "$(date -u +%FT%TZ) $*" >> "$LOG"; }
log "watching $BOX/$JOB, will delete $SID ($ZONE) after a verified complete"
for i in $(seq 1 720); do   # up to 12 h
  st=$(BOX=$BOX "$ROOT/scripts/remote.sh" ssh "tail -1 ~/archive/$JOB/run.log; grep -c FAILED ~/archive/$JOB/run.log" 2>/dev/null)
  if echo "$st" | grep -q "sha256 done"; then break; fi
  sleep 60
done
echo "$st" | grep -q "sha256 done" || { log "timeout waiting for upload — NOT deleting"; exit 1; }
fails=$(echo "$st" | tail -1)
[ "$fails" = 0 ] || { log "upload log has FAILED parts ($fails) — NOT deleting"; exit 1; }
if ! "$HERE/arch" complete --box "$BOX" --name "$JOB" >> "$LOG" 2>&1; then log "complete/verify failed — NOT deleting"; exit 1; fi
log "archive verified; deleting server $SID"
sh -c \
  "export SCW_DEFAULT_ORGANIZATION_ID=\$SCW_ORG_ID SCW_DEFAULT_PROJECT_ID=\$SCW_PROJECT_ID; scw baremetal server delete $SID zone=$ZONE" >> "$LOG" 2>&1 \
  && log "DELETED $SID" || log "delete command failed"
