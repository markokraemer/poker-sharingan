#!/bin/bash
# usage: run.sh TAG HERO FIELD DEALS SEED [extra vsbp args...]   (runs from ~/pbench; logs in logs/svan/TAG.*)
TAG=$1; HERO=$2; FIELD=$3; DEALS=$4; SEED=$5; shift 5
cd ~/pbench || exit 1
mkdir -p logs/svan/stats-$TAG
export SVAN_STATS_DIR=$PWD/logs/svan/stats-$TAG RAYON_NUM_THREADS=${THREADS:-64}
SV="ext:svan+obs=[\"/home/ubuntu/svan/target/release/extbridge\"]"
sub() { case "$1" in svan) echo "$SV";; *) echo "$1";; esac; }
H=$(sub "$HERO"); F=$(sub "$FIELD")
echo "CMD hero=$H field=$F deals=$DEALS seed=$SEED $@" > logs/svan/$TAG.log
START=$(date +%s)
./target/release/examples/vsbp --compact data/C477/compact.plcp --sizing medium --buckets data/buckets-200 --hero "$H" --field "$F" --deals $DEALS --seed $SEED --ext-workers ${WORKERS:-48} --ext-timeout-ms 120000 "$@" >> logs/svan/$TAG.log 2>&1
echo "WALL $(($(date +%s)-START)) s" >> logs/svan/$TAG.log
python3 ~/svan/tools/agg_stats.py logs/svan/stats-$TAG >> logs/svan/$TAG.log 2>&1
