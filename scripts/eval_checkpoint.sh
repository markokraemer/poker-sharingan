#!/usr/bin/env bash
# One-shot evaluation of a training box's latest blueprint checkpoint.
#
#   scripts/eval_checkpoint.sh A|B [options]
#
# 1. (box)  rsync the sources to ~/pluribus-eval and build `pluribus` there (own target dir;
#           never touches the running `pluribus-train` service or its binary);
# 2. (box)  hard-link blueprint.bin (+ snapshots.sum) and stream-compact it into a PLCP1 file
#           under a systemd unit with Nice=19, idle IO class, <= 8 cores, MemoryMax=100G;
# 3. (A->dev) copy the compact file box -> dev box DIRECTLY (rsync over ssh with agent
#           forwarding: no key is stored on any box); falls back to an ssh pipe through the
#           Mac (RAM only, never the Mac's disk);
# 4. (dev)  gauntlet vs the built-ins, h2h vs the other box's latest compact (if present) and
#           the trained-best-response exploiter, <= 16 threads, logs + JSON in data/eval/TAG.
#
# Options:
#   --run-dir DIR       run dir on the box (absolute; default /home/ubuntu/pluribus/data/run-paper for A, run-local for B)
#   --sizing S          paper|local|tiny (default: read from DIR/config.txt)
#   --min-visits N      drop never/rarely visited infosets in the compact file (default 0 = exact)
#   --deals N           gauntlet deals (default 2000)
#   --h2h-deals N      h2h deals per format (default 50000; ~1 min on 16 threads)
#   --exploit-iters N   exploiter training iterations (default 20000000 = ~2.5 h on 16 threads against the paper-size blueprint; 0 = skip)
#   --exploit-deals N   exploiter evaluation deals (default 4000)
#   --exploit-mem-gb N  learner store memory on dev (default 60)
#   --vs FILE:SIZING    h2h opponent on the dev box instead of the other box's latest
#   --skip-gauntlet | --skip-h2h | --skip-exploit | --skip-eval | --no-build | --no-compact
#   --build-only        only sync + build the binary on the box (pre-warm), then exit
#   --tag TAG           reuse an existing tag (with --no-compact: just transfer + evaluate)
#   --keep N            compact files kept per box on dev (default 2)
#   --threads N         threads for the evaluation on dev (default 16)
#   --dev-target NAME   cargo target dir name on dev (default evalpipe)
set -euo pipefail

BOX="${1:?usage: $0 A|B [options]}"; shift
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
KEY="$HOME/.ssh/pluribus_ed25519"
case "$BOX" in
  A) HOST="${PLURIBUS_HOST:?set PLURIBUS_HOST}";  RUN_DIR=/home/ubuntu/pluribus/data/run-paper ;;
  B) HOST="${PLURIBUS_HOST:?set PLURIBUS_HOST}"; RUN_DIR=/home/ubuntu/pluribus/data/run-local ;;
  *) echo "BOX must be A or B"; exit 2 ;;
esac
DEV_HOST="${PLURIBUS_DEV_HOST:?set PLURIBUS_DEV_HOST}"
SIZING=""; MIN_VISITS=0; DEALS=2000; H2H_DEALS=50000; EX_ITERS=20000000; EX_DEALS=4000; EX_MEM=60; VS=""
BUILD_ONLY=0; DO_GAUNTLET=1; DO_H2H=1; DO_EXPLOIT=1; DO_EVAL=1; DO_BUILD=1; DO_COMPACT=1; TAG=""; KEEP=2
DEV_TARGET="${DEV_TARGET:-evalpipe}"; THREADS=16
while [ $# -gt 0 ]; do
  case "$1" in
    --run-dir) RUN_DIR="$2"; shift 2 ;;
    --sizing) SIZING="$2"; shift 2 ;;
    --min-visits) MIN_VISITS="$2"; shift 2 ;;
    --deals) DEALS="$2"; shift 2 ;;
    --h2h-deals) H2H_DEALS="$2"; shift 2 ;;
    --exploit-iters) EX_ITERS="$2"; shift 2 ;;
    --exploit-deals) EX_DEALS="$2"; shift 2 ;;
    --exploit-mem-gb) EX_MEM="$2"; shift 2 ;;
    --vs) VS="$2"; shift 2 ;;
    --tag) TAG="$2"; shift 2 ;;
    --keep) KEEP="$2"; shift 2 ;;
    --dev-target) DEV_TARGET="$2"; shift 2 ;;
    --threads) THREADS="$2"; shift 2 ;;
    --skip-gauntlet) DO_GAUNTLET=0; shift ;;
    --skip-h2h) DO_H2H=0; shift ;;
    --skip-exploit) DO_EXPLOIT=0; shift ;;
    --skip-eval) DO_EVAL=0; shift ;;
    --no-build) DO_BUILD=0; shift ;;
    --no-compact) DO_COMPACT=0; shift ;;
    --build-only) BUILD_ONLY=1; shift ;;
    *) echo "unknown option $1"; exit 2 ;;
  esac
done
[ "$EX_ITERS" = 0 ] && DO_EXPLOIT=0

SSHO=(-A -i "$KEY" -o IdentitiesOnly=yes -o UserKnownHostsFile="$HOME/.ssh/known_hosts_pluribus" -o StrictHostKeyChecking=accept-new -o ServerAliveInterval=30 -o ServerAliveCountMax=20)
DEVO=(-A -i "$KEY" -o IdentitiesOnly=yes -o UserKnownHostsFile="$HOME/.ssh/known_hosts_pluribus_dev" -o StrictHostKeyChecking=accept-new -o ServerAliveInterval=30 -o ServerAliveCountMax=20)
rbox() { ssh "${SSHO[@]}" "ubuntu@$HOST" "$@"; }
rdev() { ssh "${DEVO[@]}" "ubuntu@$DEV_HOST" "$@"; }
log() { echo "[$(date -u +%H:%M:%S)] $BOX: $*"; }

# Agent forwarding needs the key in the agent (memory only).
ssh-add -t 8h "$KEY" 2>/dev/null   # (re)load with a fresh lifetime
ssh-add -l | grep -q pluribus || { echo "ssh-agent has no pluribus key"; exit 5; }

# Run a shell command as a detached, resource-limited systemd unit on the box and wait for it
# (survives ssh drops). box_job UNIT LOGFILE CPUQUOTA MEMORY CMD...  (exit code: LOGFILE.rc)
box_job() {
  local unit="$1" lg="$2" cpu="$3" mem="$4"; shift 4
  rbox "mkdir -p \$(dirname $lg); rm -f $lg.rc; cat > $lg.sh" <<<"cd /home/ubuntu/pluribus-eval; $*"
  rbox "sudo systemctl reset-failed $unit 2>/dev/null; sudo systemd-run --quiet --unit=$unit --uid=ubuntu --gid=ubuntu --collect \
    -p Nice=19 -p IOSchedulingClass=idle -p CPUQuota=$cpu -p MemoryHigh=$mem -p MemoryMax=\$(( ${mem%G} + 10 ))G -p MemorySwapMax=0 \
    -p StandardOutput=append:$lg -p StandardError=append:$lg -E HOME=/home/ubuntu -E PATH=/home/ubuntu/.cargo/bin:/usr/local/bin:/usr/bin:/bin \
    bash -c 'bash $lg.sh; echo \$? > $lg.rc'"
  while rbox "sudo systemctl is-active --quiet $unit"; do
    sleep 20
    rbox "tail -n 2 $lg" 2>/dev/null | sed "s/^/    /" >&2 || true
  done
  local rc; rc=$(rbox "cat $lg.rc 2>/dev/null || echo 99")
  [ "$rc" = 0 ] || { log "job $unit failed (rc=$rc); log tail:"; rbox "tail -n 20 $lg"; return 1; }
}

STAMP="$(date -u +%Y%m%dT%H%MZ)"
if [ -z "$TAG" ]; then
  IT="$(rbox "cat $RUN_DIR/progress.txt 2>/dev/null || echo unknown")"
  TAG="$BOX-it$IT-$STAMP"
fi
# Outside ~/pluribus: scripts/remote.sh sync runs rsync --delete on that tree (it wiped an eval dir once).
RDIR="/home/ubuntu/pluribus-eval/data/$TAG"   # on the box
DDIR="/home/ubuntu/pluribus/data/eval/$TAG"   # on dev (dev.sh sync never deletes)
# A reused --tag must keep the sizing it was compacted with (the default run dir can belong to another run:
# a medium compact once got evaluated with the paper menu -> garbage -2364 mbb/g). The tag dir on dev remembers it.
if [ -z "$SIZING" ] && [ "$DO_COMPACT" = 0 ]; then SIZING="$(rdev "cat $DDIR/sizing 2>/dev/null" || true)"; fi
[ -n "$SIZING" ] || SIZING="$(rbox "awk '/^sizing/{print \$2}' $RUN_DIR/config.txt")"
log "tag $TAG, sizing $SIZING, run dir $RUN_DIR"

if [ "$DO_COMPACT" = 1 ]; then
  if [ "$DO_BUILD" = 1 ]; then
    log "sync sources -> ~/pluribus-eval and build (niced, 4 threads)"
    rsync -az --delete -e "ssh ${SSHO[*]}" --exclude target --exclude data --exclude logs --exclude .git "$ROOT/" "ubuntu@$HOST:~/pluribus-eval/"
    box_job "eval-build-$BOX" /home/ubuntu/pluribus-eval-build.log "400%" "20G" \
      "CARGO_TARGET_DIR=/home/ubuntu/pluribus-eval/target RUSTFLAGS='-C target-cpu=native' cargo build --release -p pluribus -j 4"
  fi
  [ "$BUILD_ONLY" = 1 ] && { log "build done"; exit 0; }
  rbox "test -f $RUN_DIR/blueprint.bin" || { log "no checkpoint yet in $RUN_DIR (first one is written after --checkpoint-min)"; exit 3; }
  log "hard-link checkpoint and compact (<= 8 threads, MemoryMax 100G)"
  rbox "mkdir -p $RDIR && ln -f $RUN_DIR/blueprint.bin $RDIR/blueprint.bin && { [ -f $RUN_DIR/snapshots.sum ] && ln -f $RUN_DIR/snapshots.sum $RDIR/snapshots.sum || true; }; ls -la $RDIR"
  CMP="/home/ubuntu/pluribus-eval/target/release/pluribus compact --ckpt $RDIR/blueprint.bin --out $RDIR/compact.plcp --min-visits $MIN_VISITS --threads 8"
  rbox "[ -f $RDIR/snapshots.sum ]" && CMP="$CMP --sums $RDIR/snapshots.sum"
  T0=$(date +%s)
  box_job "eval-compact-$BOX" "$RDIR/compact.log" "800%" "90G" "$CMP > $RDIR/compact.json"
  log "compact done in $(( $(date +%s) - T0 )) s: $(rbox "cat $RDIR/compact.json")"
  rbox "rm -f $RDIR/blueprint.bin $RDIR/snapshots.sum; ls -la $RDIR"   # release the pinned inodes
fi

# ---- transfer box -> dev ------------------------------------------------------------------
rdev "mkdir -p $DDIR"
SIZE="$(rbox "stat -c %s $RDIR/compact.plcp")"
DSIZE="$(rdev "stat -c %s $DDIR/compact.plcp 2>/dev/null || echo 0")"
if [ "$SIZE" != "$DSIZE" ]; then
  rbox "ssh-add -l >/dev/null" || log "WARNING: agent forwarding to the box does not work; transfer will go through the Mac (slow)"
  log "transfer $(( SIZE / 1000000 )) MB box -> dev (direct, agent-forwarded)"
  T0=$(date +%s)
  if ! rbox "nice -n 19 ionice -c3 rsync -a --partial --inplace -e 'ssh -A -c aes128-gcm@openssh.com -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile=/dev/null' $RDIR/compact.plcp $RDIR/compact.json $RDIR/compact.log ubuntu@$DEV_HOST:pluribus/data/eval/$TAG/"; then
    log "direct rsync failed; falling back to an ssh pipe through the Mac (no local disk)"
    rbox "cat $RDIR/compact.plcp" | rdev "cat > $DDIR/compact.plcp"
  fi
  log "transfer took $(( $(date +%s) - T0 )) s"
  DSIZE="$(rdev "stat -c %s $DDIR/compact.plcp")"
  [ "$SIZE" = "$DSIZE" ] || { log "size mismatch box=$SIZE dev=$DSIZE"; exit 4; }
fi
rdev "echo '$SIZING' > $DDIR/sizing; cd ~/pluribus/data/eval && ln -sfn $TAG/compact.plcp latest-$BOX.plcp && echo '$SIZING' > latest-$BOX.sizing && echo '$TAG' > latest-$BOX.tag"
# Retention: keep the newest KEEP compact files per box on dev, 1 on the box.
rdev "cd ~/pluribus/data/eval && ls -1dt $BOX-it*/ 2>/dev/null | tail -n +$((KEEP + 1)) | xargs -r rm -rf"
rbox "cd ~/pluribus-eval/data && ls -1dt $BOX-it*/ 2>/dev/null | tail -n +2 | xargs -r rm -rf"
[ "$DO_EVAL" = 1 ] || { log "done (no eval requested): dev:$DDIR/compact.plcp"; exit 0; }

# ---- evaluation on dev (<= 16 threads) -----------------------------------------------------
log "sync + build on dev"
DEV_TARGET="$DEV_TARGET" "$ROOT/scripts/dev.sh" run 'cargo build --release -p pluribus -p bench -p exploit 2>&1 | tail -3'
OTHER=$([ "$BOX" = A ] && echo B || echo A)
{
  echo "#!/usr/bin/env bash"
  echo "set -u; cd ~/pluribus; export RAYON_NUM_THREADS=$THREADS; BIN=target/$DEV_TARGET/release; D=data/eval/$TAG; F=\$D/compact.plcp"
  echo "BK=data/buckets-200"
  if [ "$DO_GAUNTLET" = 1 ]; then
    echo "echo '== gauntlet' ; \$BIN/bench gauntlet --hero compact:\$F --bucketer tables:\$BK --sizing $SIZING --deals $DEALS > \$D/gauntlet.txt 2> \$D/gauntlet.err; cat \$D/gauntlet.txt"
  fi
  if [ "$DO_H2H" = 1 ]; then
    if [ -n "$VS" ]; then
      echo "echo '== h2h vs $VS'; \$BIN/pluribus h2h --a \$F --sizing-a $SIZING --b ${VS%%:*} --sizing-b ${VS##*:} --buckets \$BK --players 6 --deals $H2H_DEALS --threads $THREADS --warm --out \$D/h2h.json > \$D/h2h.txt 2> \$D/h2h.err; head -12 \$D/h2h.txt"
    else
      cat <<EOS
if [ -e data/eval/latest-$OTHER.plcp ]; then
  OS=\$(cat data/eval/latest-$OTHER.sizing); OT=\$(cat data/eval/latest-$OTHER.tag)
  if [ "$BOX" = A ]; then AF=\$F; AS=$SIZING; BF=data/eval/latest-$OTHER.plcp; BS=\$OS; else AF=data/eval/latest-$OTHER.plcp; AS=\$OS; BF=\$F; BS=$SIZING; fi
  echo "== h2h A=\$AF B=\$BF"
  \$BIN/pluribus h2h --a \$AF --sizing-a \$AS --b \$BF --sizing-b \$BS --buckets \$BK --players 6 --deals $H2H_DEALS --threads $THREADS --warm --out \$D/h2h-A-vs-B.json > \$D/h2h.txt 2> \$D/h2h.err; head -12 \$D/h2h.txt
else echo "== h2h skipped: no latest-$OTHER.plcp on dev yet"; fi
EOS
    fi
  fi
  if [ "$DO_EXPLOIT" = 1 ]; then
    echo "echo '== exploit-compact'; \$BIN/pluribus exploit-compact --target \$F --sizing $SIZING --buckets \$BK --players 6 --iters $EX_ITERS --threads $THREADS --mem-gb $EX_MEM --seat all --warm --eval-deals $EX_DEALS --seed 1 --out \$D/exploit.json > /dev/null 2> \$D/exploit.err; tail -3 \$D/exploit.err; python3 -I -c \"import json;d=json.load(open('\$D/exploit.json'));print(json.dumps(d['exploitation']['avg_mbb']), d['learner'])\""
  fi
  echo "touch \$D/eval.done"
} | rdev "mkdir -p ~/pluribus/logs && cat > ~/pluribus/logs/eval-$TAG.sh"
rdev "cd ~/pluribus && rm -f data/eval/$TAG/eval.done && nohup bash logs/eval-$TAG.sh > logs/eval-$TAG.log 2>&1 &"
log "evaluation running on dev (logs/eval-$TAG.log); polling"
until rdev "test -f ~/pluribus/data/eval/$TAG/eval.done"; do sleep 30; done
rdev "cat ~/pluribus/logs/eval-$TAG.log"
log "results in dev:~/pluribus/data/eval/$TAG/ (compact.json, gauntlet.txt, h2h*.json, exploit.json)"
