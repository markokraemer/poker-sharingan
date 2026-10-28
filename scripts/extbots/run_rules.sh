#!/usr/bin/env bash
# Dev box only. B-final vs rule-based bots: hero vs 5x each, plus mixed tables. Duplicate + AIVAT.
set -u
cd ~/pluribus
export RAYON_NUM_THREADS=${THREADS:-16}
B=data/eval/B-it20000000000-20261013T1354Z/compact.plcp
BIN=target/extbots/release/bench
OUT=data/extbots-results; mkdir -p $OUT
D=${DEALS:-100000}
H="--hero compact:$B --sizing local --bucketer tables:data/buckets-200"
for f in chart6 chart6-loose chart6-tight tag lag nit maniac station; do
  echo "### B vs 5x $f ($D deals)"; $BIN table $H --field $f --deals $D --seed 101 2>&1 | grep -v '^report' | tee $OUT/B-vs-5x-$f.txt
done
echo "### mixed 1: B + chart6, chart6-loose, chart6-tight, tag, lag"
$BIN table $H --field chart6,chart6-loose,chart6-tight,tag,lag --deals $D --seed 102 2>&1 | grep -v '^report' | tee $OUT/mixed-rules-1.txt
echo "### mixed 2: B + chart6, chart6-loose, nit, maniac, station"
$BIN table $H --field chart6,chart6-loose,nit,maniac,station --deals $D --seed 103 2>&1 | grep -v '^report' | tee $OUT/mixed-rules-2.txt
echo DONE
