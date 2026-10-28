#!/usr/bin/env bash
# Deploy + operate the blueprint run on the rented box.
#   scripts/remote.sh sync      # rsync code + bucket tables, build release on the box
#   scripts/remote.sh start ARGS# (re)start training service with extra pluribus-train args
#   scripts/remote.sh log       # tail training log
#   scripts/remote.sh ssh CMD   # run a command
set -euo pipefail
# Set PLURIBUS_HOST to the box address (no default). Boxes were: A = variant C (medium), B = local sizing, D = paper sizing.
HOST="${PLURIBUS_HOST:?set PLURIBUS_HOST to the training box address}"
KEY="${PLURIBUS_KEY:-$HOME/.ssh/pluribus_ed25519}"
KH="$HOME/.ssh/known_hosts_pluribus"
SSH=(ssh -i "$KEY" -o IdentitiesOnly=yes -o UserKnownHostsFile="$KH" -o StrictHostKeyChecking=accept-new -o ServerAliveInterval=30 "ubuntu@$HOST")
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
case "${1:-}" in
  sync)
    rsync -az --delete -e "ssh -i $KEY -o IdentitiesOnly=yes -o UserKnownHostsFile=$KH -o StrictHostKeyChecking=accept-new" --exclude target --exclude 'data/run-*' --exclude 'data/probe-*' --exclude logs --exclude '*.log' --exclude '*.out' --exclude '*.stop' --exclude '*.jsonl' "$ROOT/" "ubuntu@$HOST:~/pluribus/"
    "${SSH[@]}" 'set -e; command -v cargo >/dev/null || { curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null; }; source ~/.cargo/env; sudo apt-get -qq install -y build-essential tmux >/dev/null 2>&1 || true; cd ~/pluribus && RUSTFLAGS="-C target-cpu=native" cargo build --release -p pluribus 2>&1 | tail -2'
    ;;
  start)
    shift
    "${SSH[@]}" "sudo systemd-run --unit=pluribus-train --collect --property=Restart=on-failure --property=WorkingDirectory=/home/ubuntu/pluribus --uid=ubuntu --setenv=RUST_BACKTRACE=1 bash -c 'exec ./target/release/pluribus train $* --resume >> /home/ubuntu/pluribus/train.log 2>&1'"
    ;;
  stop) "${SSH[@]}" 'sudo systemctl stop pluribus-train' ;;
  log) "${SSH[@]}" 'tail -n ${N:-20} ~/pluribus/train.log' ;;
  ssh) shift; "${SSH[@]}" "$@" ;;
  *) echo "usage: $0 sync|start ARGS|stop|log|ssh CMD" ;;
esac
