#!/usr/bin/env bash
# Runs ON the training box (no credentials needed). Waits for the C run to reach its target,
# then compacts the final blueprint and evaluates it on the same box (no separate eval box):
#   gauntlet vs built-ins, h2h vs C@4.77e10 (4M deals/format), trained-best-response exploiter.
# Writes ~/finish/DONE (or ~/finish/FAILED). The Mac watcher (finish_c_watch.sh) then archives + deletes.
#   usage: box_finish.sh TARGET_ITERS          log: ~/finish/finish.log
set -u
TARGET="$1"
RUN=/home/ubuntu/pluribus/data/run-medium
OUT=/home/ubuntu/finish
REF=/home/ubuntu/ref/C-medium-4.77e10.plcp
BIN=/home/ubuntu/pluribus/target/release
BK=/home/ubuntu/pluribus/data/buckets-200
mkdir -p "$OUT"; cd /home/ubuntu/pluribus || exit 1
log() { echo "$(date -u +%FT%TZ) $*" >> "$OUT/finish.log"; }
fail() { log "FAILED: $*"; touch "$OUT/FAILED"; exit 1; }

log "waiting for training to reach $TARGET"
while :; do
  IT=$(cat "$RUN/progress.txt" 2>/dev/null || echo 0)
  if ! systemctl is-active --quiet pluribus-train; then
    [ "$IT" -ge "$TARGET" ] 2>/dev/null && break
    sleep 600   # Restart=on-failure may be restarting it; only give up if it stays down
    systemctl is-active --quiet pluribus-train || { IT=$(cat "$RUN/progress.txt"); [ "$IT" -ge "$TARGET" ] 2>/dev/null && break; fail "training stopped at $IT < $TARGET"; }
  fi
  sleep 300
done
log "training done at $(cat $RUN/progress.txt); building eval binaries"
source ~/.cargo/env
RUSTFLAGS="-C target-cpu=native" cargo build --release -p pluribus -p bench -p exploit >> "$OUT/build.log" 2>&1 || fail "build"

log "compact"
"$BIN/pluribus" compact --ckpt "$RUN/blueprint.bin" --sums "$RUN/snapshots.sum" --out "$OUT/C-medium-final.plcp" \
  --min-visits 0 --threads 64 > "$OUT/compact.json" 2> "$OUT/compact.log" || fail "compact"
log "compact: $(cat $OUT/compact.json)"
echo medium > "$OUT/sizing"

export RAYON_NUM_THREADS=120
log "gauntlet"
"$BIN/bench" gauntlet --hero compact:"$OUT/C-medium-final.plcp" --bucketer tables:$BK --sizing medium --deals 20000 \
  > "$OUT/gauntlet.txt" 2> "$OUT/gauntlet.err" || log "gauntlet failed (continuing)"

if [ -f "$REF" ]; then
  log "h2h final vs C@4.77e10"
  "$BIN/pluribus" h2h --a "$OUT/C-medium-final.plcp" --sizing-a medium --b "$REF" --sizing-b medium --buckets $BK \
    --players 6 --deals 4000000 --threads 120 --warm --out "$OUT/h2h-final-vs-C477.json" > "$OUT/h2h.txt" 2> "$OUT/h2h.err" \
    || log "h2h failed (continuing)"
else
  log "h2h skipped: $REF missing"
fi

log "exploit-compact"
"$BIN/pluribus" exploit-compact --target "$OUT/C-medium-final.plcp" --sizing medium --buckets $BK --players 6 \
  --iters 12000000 --threads 120 --mem-gb 120 --seat all --warm --eval-deals 20000 --seed 1 \
  --out "$OUT/exploit.json" > /dev/null 2> "$OUT/exploit.err" || log "exploit failed (continuing)"

log "all evals finished"
touch "$OUT/DONE"
