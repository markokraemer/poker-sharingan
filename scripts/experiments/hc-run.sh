#!/usr/bin/env bash
# On box B: run named arms sequentially.  usage: hc-run.sh DEALS SEED name1='--set ...' name2=...
# (run from ~/pluribus-search)
cd ~/${DEST:-pluribus-search}
D=$1; S=$2; shift 2
for arm in "$@"; do
  name=${arm%%=*}; sets=${arm#*=}
  [ -s logs/hc/$name.log ] && grep -q RESULT logs/hc/$name.log && continue
  RAYON_NUM_THREADS=${THREADS:-128} target/release/examples/vsbp --compact data/Bfinal/compact.plcp --hero search --field B --deals $D --seed $S --set "$sets" > logs/hc/$name.log 2>&1
done
