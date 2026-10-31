#!/bin/sh
# Isolation check against the REAL bot image (run on a host with podman, as root): a sandbox-like container (no network,
# tmpfs scratch, init, no-new-privileges) plays a short run with test_isolation_cheater.py, a bot that tries to carry
# notes from one rotation of duplicate deals into the next. With --rotation-uids no note may cross from one rotation's
# user to another's (lanes of one rotation share a user and different deals: that is fine); without it (all rotations as
# one user, no scrub) the same bot must find notes of earlier processes, which proves the probe works.
#   sudo sh deploy/platinum/bot/test_isolation.sh localhost/sharingan-bot:candidate /srv/sharingan/models
# MODELS holds sharingan-1.plcp and buckets-200 (every track seats SHARINGAN-1).
set -u
IMG="${1:?usage: test_isolation.sh IMAGE MODELS_DIR}"
MODELS="${2:?usage: test_isolation.sh IMAGE MODELS_DIR}"
HERE=$(cd "$(dirname "$0")" && pwd)
C=sharingan-isolation-test
podman rm -f "$C" >/dev/null 2>&1
podman run -d --name "$C" --network none --memory 4g --pids-limit 512 --security-opt no-new-privileges --init \
  --security-opt apparmor="${APPARMOR:-sharingan-sandbox}" \
  --tmpfs /tmp:rw,nosuid,nodev,size=2g,mode=1777 --tmpfs /var/tmp:rw,nosuid,nodev,size=256m,mode=1777 \
  --tmpfs /opt/sharingan/bot:rw,nosuid,nodev,size=1g,mode=0755 --tmpfs /opt/sharingan/deps:rw,nosuid,nodev,size=2g,mode=0755 \
  -v "$MODELS:/models:ro" -e ARENA_BLUEPRINT=/models/sharingan-1.plcp -e ARENA_BUCKETS=tables:/models/buckets-200 -e ARENA_SIZING=medium \
  "$IMG" sleep infinity >/dev/null || exit 1
podman cp "$HERE/test_isolation_cheater.py" "$C:/opt/sharingan/bot/bot.py" && podman exec "$C" sh -c 'chown root:root /opt/sharingan/bot/bot.py && chmod 0644 /opt/sharingan/bot/bot.py'
BOT='env -i HOME=/tmp PATH=/usr/local/bin:/usr/bin:/bin PYTHONPATH=/opt/sharingan/deps PYTHONUNBUFFERED=1 python3 /opt/sharingan/bot/bot.py'
run() {  # $1 = label, rest = extra worker args
  label=$1; shift
  podman exec "$C" sh -c "mkdir -p /work && cd /work && sharingan-arena run --bot \"\$0\" --track ladder-v1 --hands 1200 --lanes 2 --out /work/r.json --quiet $*" "$BOT" 2>/tmp/iso-$label.err
  echo "$label: exit $? | hands $(podman exec "$C" python3 -c 'import json; r=json.load(open("/work/r.json")); print(r["hands"], "isolated:", r["worker"].get("isolated_rotations"))')"
  # notes seen across users (rotations), and notes seen at all
  echo "  crossed between users: $(awk '/^SEEN/ { split($(NF-2), a, ":"); split($NF, b, ":"); if (a[1] != b[1]) n++ } END { print n+0 }' /tmp/iso-$label.err) | seen from other processes: $(grep -c '^SEEN' /tmp/iso-$label.err)"
}
run isolated --rotation-uids 10010,10011,10012,10013,10014,10015
awk '/^SEEN/ { split($(NF-2), a, ":"); split($NF, b, ":"); if (a[1] != b[1]) print }' /tmp/iso-isolated.err | head
podman exec "$C" sh -c 'for u in 10010 10011 10012 10013 10014 10015; do n=$(find / -xdev -user $u 2>/dev/null | wc -l); [ "$n" = 0 ] || echo "files of $u left: $n"; done; ps -eo user,pid,cmd 2>/dev/null | grep -c rot || true'
# control: every rotation as the same user and no scrub -> the cheater must see its notes
podman exec "$C" sh -c 'sharingan-scrub 10002 >/dev/null 2>&1; true'
BOT="exec setpriv --reuid=10002 --regid=10002 --clear-groups $BOT"
run control
grep '^SEEN' /tmp/iso-control.err | awk '{ $(NF-2)=""; $NF=""; print }' | sort | uniq -c | head
podman rm -f "$C" >/dev/null
