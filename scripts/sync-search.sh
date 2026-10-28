#!/usr/bin/env bash
# Copy the workspace SOURCE (no data/target/logs) to box B ~/$DEST (default pluribus-search) and build the search examples there.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
D="${DEST:-pluribus-search}"
HOST="${PLURIBUS_HOST:?set PLURIBUS_HOST to the box address}"; KEY="$HOME/.ssh/pluribus_ed25519"; KH="$HOME/.ssh/known_hosts_pluribus"
rsync -az -e "ssh -i $KEY -o IdentitiesOnly=yes -o UserKnownHostsFile=$KH" --exclude target --exclude data --exclude logs --exclude .git "$ROOT/" "ubuntu@$HOST:~/$D/"
PLURIBUS_HOST="$HOST" BOX=B "$ROOT/scripts/remote.sh" ssh "source ~/.cargo/env; cd ~/$D && mkdir -p logs/hc data && ln -sfn ~/pluribus-search/data/Bfinal data/Bfinal && ln -sfn ~/pluribus-search/data/buckets-200 data/buckets-200 && RUSTFLAGS='-C target-cpu=native' cargo build --release -p search --examples 2>&1 | tail -15"
