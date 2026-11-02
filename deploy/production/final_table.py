#!/usr/bin/env python3
"""The competition's final table (COMPETITION.md, step 3), run by the organizers as root on the arena host.

Each finalist plays its exact upload in a sandbox of its own: the bot image, the sharingan-sandbox AppArmor profile, no
capabilities, 4 CPU / 12 GB, unpacked and sealed like a hosted run (deploy/platinum/runner/src/job.ts). It connects over
WebSocket to the arena on a network made for the final that reaches nothing but the arena. Seats without a finalist are
played by house:sharingan-1. Names are hidden: finalists play as finalist-1..6 (assigned at random). One duplicate match,
every deal from every seat, other players' cards withheld until the match ends. The highest luck-corrected (AIVAT)
mbb/game among the finalists wins.

  sudo python3 deploy/production/final_table.py --bots DIR --out DIR [--hands 999996] [--lanes 12] [--cleanup]

  --bots DIR    one <label>.tgz per finalist: the job artifact (GET /v1/jobs/<id>/artifact; check its sha256 against
                the job's artifact_sha256). Optional <label>.entry (entry file, default bot.py) and <label>.req
                (requirements file inside the archive, installed unprivileged in a short network window).
  --out DIR     writes finalists.json (label -> anonymous name, player id, upload sha256), match.json (standings),
                hands.jsonl (every hand), logs/<name>.err (each bot's stderr tail).
  --cleanup     delete the finalist identities afterwards (and with them the match): for dry runs.
"""
import argparse
import hashlib
import json
import os
import random
import shlex
import subprocess
import sys
import time
import urllib.request

ARENA = "http://127.0.0.1:8080"                       # the arena as the host reaches it
BOT_URL = "http://sharingan-core:8080"                # the arena as a finalist's sandbox reaches it
IMAGE = "localhost/sharingan-bot:prod"
NET = "sharingan-final"
CORE = "sharingan-core"
BOT_DIR, DEPS_DIR = "/opt/sharingan/bot", "/opt/sharingan/deps"
TMPFS = {"/tmp": "size=2g,mode=1777", "/var/tmp": "size=256m,mode=1777", BOT_DIR: "size=1g,mode=0755",
         DEPS_DIR: "size=2g,mode=0755", "/opt/sharingan/run": "size=64m,mode=0700"}
DROP = "setpriv --reuid=10002 --regid=10002 --clear-groups --inh-caps=-all --bounding-set=-all"
PATHS = "PATH=/usr/local/bin:/usr/bin:/bin"


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, flush=True)


def sh(*cmd, check=True, input=None):
    r = subprocess.run(cmd, capture_output=True, text=True, input=input)
    if check and r.returncode:
        raise SystemExit(f"failed: {' '.join(cmd)}\n{r.stderr.strip()[-600:]}")
    return r


def ex(c, script, check=True):
    return sh("podman", "exec", c, "sh", "-c", script, check=check)


def api(method, path, body=None):
    token = open("/srv/sharingan/secrets/arena_admin_token").read().strip()
    req = urllib.request.Request(ARENA + path, method=method, data=None if body is None else json.dumps(body).encode(),
                                 headers={"authorization": f"Bearer {token}", "content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as r:
        raw = r.read()
    return raw if path.endswith(".jsonl") else json.loads(raw)


def sandbox(name, tgz, entry, req):
    """A finalist's container: same limits, profile and unpack/seal steps as a hosted run."""
    sh("podman", "rm", "-f", name, check=False)
    args = ["podman", "run", "-d", "--name", name, "--network", NET, "--memory", "12g", "--cpus", "4", "--pids-limit", "512",
            "--security-opt", "no-new-privileges", "--security-opt", "apparmor=sharingan-sandbox", "--init", "--ulimit", "core=0"]
    for path, opts in TMPFS.items():
        args += ["--tmpfs", f"{path}:rw,nosuid,nodev,{opts}"]
    sh(*args, IMAGE, "sleep", "infinity")
    sh("podman", "cp", tgz, f"{name}:/tmp/bot.tgz")
    ex(name, f"chown 10002:10002 {BOT_DIR} && chmod 0644 /tmp/bot.tgz && {DROP} tar xzf /tmp/bot.tgz -C {BOT_DIR} "
             f"--no-same-owner --no-same-permissions && rm -f /tmp/bot.tgz && chown -R root:root {BOT_DIR} "
             f"&& chmod -R a+rX,go-w {BOT_DIR} && chmod 0755 {BOT_DIR}")
    ex(name, f"test -f {shlex.quote(BOT_DIR + '/' + entry)}")
    if req:
        sh("podman", "network", "connect", "sharingan-egress", name)
        try:
            ex(name, f"cp -r {BOT_DIR} /tmp/pip-src && chown -R 10002:10002 {DEPS_DIR} /tmp/pip-src && cd /tmp/pip-src && {DROP} "
                     f"env -i HOME=/tmp {PATHS} pip install --no-cache-dir --disable-pip-version-check --no-warn-script-location "
                     f"--target {DEPS_DIR} -r {shlex.quote('/tmp/pip-src/' + req)}")
        finally:
            sh("podman", "network", "disconnect", "sharingan-egress", name)
        ex(name, f"sharingan-scrub 10002 --keep {DEPS_DIR} && chown -R root:root {DEPS_DIR} && chmod -R a+rX,go-w {DEPS_DIR}")
    ex(name, f"find -P {BOT_DIR} {DEPS_DIR} -exec touch -a -h -d 2099-01-01 {{}} +")   # reads leave no access-time mark
    nets = ex(name, "ls /sys/class/net").stdout.split()
    if set(nets) - {"lo", "eth0"} or "eth0" not in nets:
        raise SystemExit(f"{name}: unexpected network {nets}")


def connect(name, entry, token):
    bot = (f"setpriv --reuid=10010 --regid=10010 --clear-groups --inh-caps=-all --bounding-set=-all env -i HOME=/tmp {PATHS} "
           f"PYTHONPATH={DEPS_DIR} PYTHONUNBUFFERED=1 PYTHONDONTWRITEBYTECODE=1 sharingan connect {shlex.quote(BOT_DIR + '/' + entry)} "
           f"--server {BOT_URL} --token {token}")
    sh("podman", "exec", "-d", name, "sh", "-c",
       f"umask 077; mkdir -p -m 0700 /opt/sharingan/run/proc && exec {bot} > /opt/sharingan/run/proc/out 2> /opt/sharingan/run/proc/err")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--bots", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--hands", type=int, default=999_996)
    ap.add_argument("--lanes", type=int, default=12)
    ap.add_argument("--cleanup", action="store_true")
    a = ap.parse_args()
    if os.geteuid() != 0:
        raise SystemExit("run as root (podman and the admin token)")
    if a.hands % 6:
        raise SystemExit("--hands must be a multiple of 6 (one deal = 6 hands)")
    labels = sorted(f[:-4] for f in os.listdir(a.bots) if f.endswith(".tgz"))
    if not 1 <= len(labels) <= 6:
        raise SystemExit(f"need 1..6 finalists in {a.bots}, found {len(labels)}")
    os.makedirs(os.path.join(a.out, "logs"), exist_ok=True)
    random.SystemRandom().shuffle(labels)   # anonymous names in a random order

    if sh("podman", "network", "exists", NET, check=False).returncode:
        sh("podman", "network", "create", "--internal", NET)
    sh("podman", "network", "connect", NET, CORE, check=False)
    finalists, containers = [], []
    try:
        for i, label in enumerate(labels, 1):
            tgz = os.path.join(a.bots, label + ".tgz")
            opt = lambda ext, d: (open(os.path.join(a.bots, label + ext)).read().strip() if os.path.exists(os.path.join(a.bots, label + ext)) else d)
            entry, req = opt(".entry", "bot.py"), opt(".req", "")
            name = f"finalist-{i}"
            reg = api("POST", "/v1/bots", {"name": name})
            c = f"sharingan-final-{i}"
            containers.append(c)
            log(f"{name}: sandbox for {label}")
            sandbox(c, tgz, entry, req)
            connect(c, entry, reg["token"])
            finalists.append({"label": label, "name": name, "player_id": reg["player_id"],
                              "sha256": hashlib.sha256(open(tgz, "rb").read()).hexdigest(), "entry": entry, "requirements": req or None})
        json.dump(finalists, open(os.path.join(a.out, "finalists.json"), "w"), indent=1)

        for f, c in zip(finalists, containers):   # every bot connected before the first deal
            for _ in range(120):
                if "connected as" in ex(c, "cat /opt/sharingan/run/proc/err", check=False).stdout:
                    break
                time.sleep(1)
            else:
                raise SystemExit(f"{f['name']} never connected: {ex(c, 'tail -20 /opt/sharingan/run/proc/err', check=False).stdout}")
        log("all finalists connected")

        seats = [f["player_id"] for f in finalists] + ["house:sharingan-1"] * (6 - len(finalists))
        m = api("POST", "/v1/matches", {"participants": seats, "hands_planned": a.hands, "duplicate": True, "lanes": a.lanes,
                                       "reveal": "match_end", "aivat": "full", "table_config": {"stack_mode": "reset"}})
        mid = m["match_id"]
        api("POST", f"/v1/matches/{mid}/start")
        log(f"match {mid} started: {a.hands} hands on {a.lanes} tables")
        t0 = time.time()
        while True:
            time.sleep(30)
            m = api("GET", f"/v1/matches/{mid}")
            log(f"{m['status']} {m['hands_played']}/{a.hands} hands ({m['hands_played'] / max(1, time.time() - t0):.0f}/s)")
            if m["status"] in ("finished", "aborted"):
                break
        json.dump(m, open(os.path.join(a.out, "match.json"), "w"), indent=1)
        open(os.path.join(a.out, "hands.jsonl"), "wb").write(api("GET", f"/v1/matches/{mid}/hands.jsonl"))

        names = {f["player_id"]: f for f in finalists}
        rows = sorted((s for s in m["standings"] if s["player_id"] in names), key=lambda s: -s["mbb_per_game"])
        log(f"match {m['status']}. Standings (finalists, luck-corrected mbb/game):")
        for rank, s in enumerate(rows, 1):
            f = names[s["player_id"]]
            print(f"  {rank}. {f['name']} ({f['label']}): {s['mbb_per_game']:+.1f} +- {s['ci95_mbb']:.1f} over {s['hands']} hands ({s['method']})")
    finally:
        for c in containers:
            err = ex(c, "tail -c 20000 /opt/sharingan/run/proc/err", check=False).stdout
            open(os.path.join(a.out, "logs", c.replace("sharingan-final-", "finalist-") + ".err"), "w").write(err)
            sh("podman", "rm", "-f", c, check=False)
        sh("podman", "network", "disconnect", NET, CORE, check=False)
        sh("podman", "network", "rm", NET, check=False)
        if a.cleanup:
            for f in finalists:
                api("DELETE", f"/v1/bots/{f['player_id']}")
            log(f"deleted {len(finalists)} finalist identities (dry run)")


if __name__ == "__main__":
    sys.exit(main())
