#!/bin/bash
# Nightly logical backup of both databases (arena + runner) to /srv/sharingan/backups, 14 days kept. Run by sharingan-backup.timer.
set -euo pipefail
D=/srv/sharingan/backups; TS=$(date -u +%Y%m%dT%H%M%SZ)
mkdir -p "$D"
for db in arena runner; do
  podman exec sharingan-postgres pg_dump -U sharingan -Fc "$db" > "$D/$db-$TS.dump.tmp"
  mv "$D/$db-$TS.dump.tmp" "$D/$db-$TS.dump"
done
find "$D" -name '*.dump' -mtime +14 -delete
echo "backup ok $TS"
