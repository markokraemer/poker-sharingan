#!/usr/bin/env bash
# Mac-side, detached: the dev box is deleted only after EVERY dev archive job has completed + verified.
#   usage: finish_dev_then_delete.sh SERVER_ID ZONE JOB_ALREADY_WATCHED JOB_TO_COMPLETE...
# JOB_ALREADY_WATCHED: a job another watcher (finish_only.sh) completes; we wait for its "COMPLETE OK".
set -u
SID="$1"; ZONE="$2"; WATCHED="$3"; shift 3
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../../.." && pwd)"
LOG="$HERE/state/dev-delete.finish.log"; log() { echo "$(date -u +%FT%TZ) $*" >> "$LOG"; }
log "dev delete chain: wait $WATCHED + complete $*; server $SID"
for i in $(seq 1 720); do grep -q "COMPLETE OK" "$HERE/state/$WATCHED.finish.log" 2>/dev/null && break
  grep -q "FAILED" "$HERE/state/$WATCHED.finish.log" 2>/dev/null && { log "$WATCHED failed — NOT deleting"; exit 1; }; sleep 60; done
grep -q "COMPLETE OK" "$HERE/state/$WATCHED.finish.log" || { log "timeout on $WATCHED — NOT deleting"; exit 1; }
for J in "$@"; do
  for i in $(seq 1 240); do "$ROOT/scripts/dev.sh" sh "tail -1 ~/archive/$J/run.log" 2>/dev/null | grep -q "sha256 done" && break; sleep 60; done
  if ! "$HERE/arch" complete --box dev --name "$J" >> "$LOG" 2>&1; then log "$J complete/verify failed — NOT deleting"; exit 1; fi
  log "$J verified"
done
log "all dev archives verified; deleting dev server $SID"
sh -c \
  "export SCW_DEFAULT_ORGANIZATION_ID=\$SCW_ORG_ID SCW_DEFAULT_PROJECT_ID=\$SCW_PROJECT_ID; scw baremetal server delete $SID zone=$ZONE" >> "$LOG" 2>&1 \
  && log "DELETED $SID" || log "delete command failed"
