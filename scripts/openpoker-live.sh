#!/usr/bin/env bash
# Live Open Poker bot: C-final (2e10) + real-time search (turn+river only: flop=0), restarted on crash (panic=abort build).
cd ~/pluribus
set -a; . ./.env.openpoker; set +a
n=0
while true; do
  n=$((n+1)); echo "=== start #$n $(date -u)" >> logs/openpoker-CsearchTR2.out
  nice -n 5 ./target/opsearch-live2/openpoker live \
    --policy search:data/eval/A-it20000000000-20261014T0959Z/compact.plcp \
    --sizing medium --buckets data/buckets-200 --search-threads 16 --search-budget-ms 5000 --set flop=0 \
    --hands 100000 --buy-in 2000 --timeout-sec 172800 --no-auto-rebuy --max-stack-bb 150 \
    --trace logs/openpoker-CsearchTR2.jsonl >> logs/openpoker-CsearchTR2.out 2>&1
  rc=$?; echo "=== exit rc=$rc $(date -u)" >> logs/openpoker-CsearchTR2.out
  [ -f logs/openpoker-CsearchTR2.stop ] && break
  sleep 30
done
