#!/bin/bash
# Run on the dev box: DEV_TARGET=search scripts/dev.sh run 'nohup crates/search/run_experiments.sh > logs/search-exp.log 2>&1 &'
export RAYON_NUM_THREADS=32
cd ~/pluribus
E=./target/search/release/examples/eval
mkdir -p data/search-bp
H=data/search-bp/hu_tiny_3m.bin
S=data/search-bp/p6_tiny_20m.bin
run() { echo "### $*"; $E "$@" 2>&1 | grep -vE "^  it |^training|^loading"; }
run --players 2 --bp-cache $H --bp-iters 3000000 --deals 20000 --search-iters 5000 --no-lbr
run --players 2 --bp-cache $H --deals 20000 --search-iters 5000 --average --no-lbr
run --players 2 --bp-cache $H --deals 20000 --search-iters 20000 --average --no-lbr
run --players 2 --bp-cache $H --deals 20000 --search-iters 20000 --no-lbr
run --players 2 --bp-cache $H --deals 20000 --search-iters 5000 --leaf blueprint --no-lbr
run --players 2 --bp-cache $H --deals 1000 --search-iters 20000 --average --par-search --no-lbr
run --players 2 --bp-cache $H --deals 1 --search-iters 20000 --average --lbr-deals 1500 --lbr-samples 300
run --players 6 --bp-cache $S --bp-iters 20000000 --deals 3000 --search-iters 5000 --average --hero-only-seat0 --no-lbr
run --players 6 --bp-cache $S --deals 1500 --search-iters 5000 --average --no-lbr
run --players 6 --bp-cache $S --deals 1000 --search-iters 20000 --average --par-search --no-lbr
echo DONE
