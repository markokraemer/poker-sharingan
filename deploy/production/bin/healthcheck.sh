#!/bin/bash
# Local health summary: units, containers, arena/serve/runner probes, disk, RAM. Exit 1 if anything is down. Cron/monitor friendly.
rc=0
for u in sharingan-postgres sharingan-core sharingan-runner caddy; do
  systemctl is-active --quiet $u || { echo "DOWN unit $u"; rc=1; }
done
curl -fsS -m 5 http://127.0.0.1:8080/readyz >/dev/null || { echo "DOWN arena readyz"; rc=1; }
curl -fsS -m 5 http://127.0.0.1:8081/v1/health >/dev/null || { echo "DOWN serve health"; rc=1; }
curl -fsS -m 5 http://127.0.0.1:8090/readyz >/dev/null || { echo "DOWN runner readyz"; rc=1; }
df -h / | tail -1 | awk '{ if ($5+0 > 85) { print "WARN disk " $5; exit 1 } }' || rc=1
echo "sandboxes running: $(podman ps -q --filter label=sharingan.sandbox=1 | wc -l)"
[ $rc = 0 ] && echo OK
exit $rc
