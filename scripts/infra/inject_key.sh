#!/usr/bin/env bash
# Install ~/.ssh/pluribus_ed25519.pub on a freshly installed Scaleway Elastic Metal box whose
# project SSH keys we don't hold: reboot to rescue, mount the root RAID, append the key,
# reboot normal. Usage: scripts/infra/inject_key.sh SERVER_ID ZONE IP
set -euo pipefail
ID=$1 ZONE=$2 IP=$3
# Needs the scw CLI and SCW_ORG_ID / SCW_PROJECT_ID (+ API credentials) exported in the environment.
scw_() { sh -c "export SCW_DEFAULT_ORGANIZATION_ID=\$SCW_ORG_ID SCW_DEFAULT_PROJECT_ID=\$SCW_PROJECT_ID; scw $*"; }
get() { scw_ "baremetal server get $ID zone=$ZONE -o json" 2>/dev/null; }
until get | python3 -c "import sys,json; d=json.load(sys.stdin); sys.exit(0 if (d.get('install') or {}).get('status')=='completed' and d['status']=='ready' else 1)"; do sleep 20; done
# Let the first normal boot finish: cloud-init creates the `ubuntu` user there.
sleep 240
scw_ "baremetal server reboot $ID boot-type=rescue zone=$ZONE" >/dev/null 2>&1
until get | python3 -c "import sys,json; d=json.load(sys.stdin); sys.exit(0 if d['status']=='ready' and d.get('boot_type')=='rescue' and (d.get('rescue_server') or {}).get('password') else 1)"; do sleep 15; done
PWF=$(mktemp); chmod 600 "$PWF"
get | python3 -c "import sys,json; print(json.load(sys.stdin)['rescue_server']['password'], end='')" > "$PWF"
PUB=$(cat ~/.ssh/pluribus_ed25519.pub)
for i in $(seq 1 40); do
  sshpass -f "$PWF" ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=8 -o PreferredAuthentications=password -o PubkeyAuthentication=no rescue@$IP \
    "set -e; for d in \$(lsblk -lno NAME,TYPE | awk '\$2 ~ /raid/ {print \$1}'); do sudo mkdir -p /mnt/r; sudo mount /dev/\$d /mnt/r 2>/dev/null || continue; sudo install -d -m 700 /mnt/r/root/.ssh; echo '$PUB' | sudo tee -a /mnt/r/root/.ssh/authorized_keys >/dev/null; sudo chmod 600 /mnt/r/root/.ssh/authorized_keys; if [ -d /mnt/r/home/ubuntu ]; then U=\$(sudo stat -c %u:%g /mnt/r/home/ubuntu); sudo install -d -m 700 /mnt/r/home/ubuntu/.ssh; echo '$PUB' | sudo tee -a /mnt/r/home/ubuntu/.ssh/authorized_keys >/dev/null; sudo chown -R \$U /mnt/r/home/ubuntu/.ssh; sudo chmod 600 /mnt/r/home/ubuntu/.ssh/authorized_keys; sync; sudo umount /mnt/r; echo injected-into-\$d; exit 0; fi; sudo umount /mnt/r; done; exit 1" && break
  sleep 15
done
rm -f "$PWF"
scw_ "baremetal server reboot $ID boot-type=normal zone=$ZONE" >/dev/null 2>&1
sleep 60
for i in $(seq 1 40); do ssh -i ~/.ssh/pluribus_ed25519 -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile=$HOME/.ssh/known_hosts_pluribus_$IP -o ConnectTimeout=8 ubuntu@$IP 'echo OK $(nproc) cores; free -g | sed -n 2p' 2>/dev/null && exit 0; sleep 20; done
echo "key login failed"; exit 1
