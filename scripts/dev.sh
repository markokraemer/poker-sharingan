#!/usr/bin/env bash
# Run anything on the DEV box (never on the Mac). The Mac is the source of truth for files;
# every call rsyncs the workspace up first, then runs the command in ~/pluribus there.
#   scripts/dev.sh run 'cargo test --release -p search'      # sync + run
#   scripts/dev.sh sh  'ls data'                             # run without sync
#   scripts/dev.sh pull data/some-results.json               # copy a file back
# Set DEV_TARGET=<name> to get a private cargo target dir (avoids build-lock waits between
# parallel agents). Long jobs: wrap in `nohup ... > logs/x.log 2>&1 &` and poll.
set -euo pipefail
HOST="${PLURIBUS_DEV_HOST:?set PLURIBUS_DEV_HOST to the dev/eval box address}"
KEY="$HOME/.ssh/pluribus_ed25519"
KH="$HOME/.ssh/known_hosts_pluribus_dev"
SSHO=(-i "$KEY" -o IdentitiesOnly=yes -o UserKnownHostsFile="$KH" -o StrictHostKeyChecking=accept-new -o ServerAliveInterval=30)
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TGT="${DEV_TARGET:-main}"
sync() {
  rsync -az -e "ssh ${SSHO[*]}" --exclude target --exclude 'data/run-*' --exclude 'data/probe-*' --exclude logs --exclude '.git' "$ROOT/" "ubuntu@$HOST:~/pluribus/"
}
remote() {
  ssh "${SSHO[@]}" "ubuntu@$HOST" "source ~/.cargo/env 2>/dev/null; cd ~/pluribus && mkdir -p logs && export CARGO_TARGET_DIR=~/pluribus/target/$TGT RUSTFLAGS='-C target-cpu=native'; $1"
}
case "${1:-}" in
  run) sync; remote "$2" ;;
  sh) remote "$2" ;;
  sync) sync ;;
  pull) rsync -az -e "ssh ${SSHO[*]}" "ubuntu@$HOST:~/pluribus/$2" "$ROOT/$2" ;;
  setup)
    sync
    ssh "${SSHO[@]}" "ubuntu@$HOST" 'set -e; sudo apt-get -qq update >/dev/null; sudo apt-get -qq install -y build-essential pkg-config libssl-dev >/dev/null; command -v cargo >/dev/null || curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null; source ~/.cargo/env; rustup component add rustfmt clippy >/dev/null 2>&1 || true; cargo --version'
    ;;
  *) echo "usage: $0 run CMD | sh CMD | sync | pull PATH | setup"; exit 1 ;;
esac
