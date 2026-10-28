#!/usr/bin/env bash
# Dev box only. B-final / chart6 / builtins vs the RLCard DQN (Python bridge).
set -u
cd ~/pluribus
export RAYON_NUM_THREADS=${THREADS:-16}
B=data/eval/B-it20000000000-20261013T1354Z/compact.plcp
BIN=target/extbots/release/bench
OUT=data/extbots-results; mkdir -p $OUT
M=$HOME/extbots/rlcard-models/dqn6-final.pt
cp -n $HOME/extbots/rlcard-models/dqn6 $M
D=${DEALS:-20000}
X="--ext-workers 16 --ext-timeout-ms 10000"
H="--hero compact:$B --sizing local --bucketer tables:data/buckets-200"
echo "### B vs 5x rlcard ($D deals)"; $BIN table $H --field rlcard:$M --deals $D --seed 201 $X 2>&1 | tee $OUT/B-vs-5x-rlcard.txt | grep -v '^$'
echo "### mixed 3: B + chart6, chart6-loose, chart6-tight, rlcard, tag ($D deals)"
$BIN table $H --field chart6,chart6-loose,chart6-tight,rlcard:$M,tag --deals $D --seed 202 $X 2>&1 | tee $OUT/mixed-rlcard.txt | grep -v '^$'
echo "### chart6 vs 5x rlcard (5000 deals)"; $BIN table --hero chart6 --field rlcard:$M --deals 5000 --seed 203 $X 2>&1 | tee $OUT/chart6-vs-5x-rlcard.txt | grep -v '^$'
for f in random station tag lag; do
  echo "### rlcard vs 5x $f (3000 deals)"; $BIN table --hero rlcard:$M --field $f --deals 3000 --seed 204 $X 2>&1 | tee $OUT/rlcard-vs-5x-$f.txt | grep -v '^$'
done
echo DONE
