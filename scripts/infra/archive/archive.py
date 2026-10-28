#!/usr/bin/env python3
"""Archive box files to a private S3-compatible (Scaleway) bucket WITHOUT putting any key on a box.

The Mac holds the credentials. It creates S3 multipart uploads, presigns one PUT URL per 1 GiB
part (valid 7 days) and copies the plan to the box. The box streams the byte ranges with curl
(scripts/infra/archive/upload_parts.sh) and records each part's ETag. The Mac then completes the
uploads and verifies the object sizes.

Run under the Scaleway env (SCW_ACCESS_KEY / SCW_SECRET_KEY), e.g.
  python3 scripts/infra/archive/archive.py ...

  plan     --box A|B|D|dev --name JOB --class STANDARD|GLACIER  SRC=KEY [SRC=KEY ...]
  start    --box X --name JOB          copy plan + uploader to the box and start it (nohup)
  status   --box X --name JOB
  complete --box X --name JOB          complete the multipart uploads + verify sizes
  fetch    --box X --name JOB KEY=DEST ...   presign GETs, box downloads them with aria2c (GLACIER: restore first)
  fetched  --box X --name JOB          check the download finished and every size matches
"""
import argparse, json, os, subprocess, sys, time
from pathlib import Path

import boto3
from botocore.config import Config

BUCKET = os.environ.get("ARCHIVE_BUCKET", "<your-private-bucket>")
ENDPOINT = os.environ.get("S3_ENDPOINT", "https://s3.<region>.scw.cloud")
REGION = os.environ.get("S3_REGION", "<region>")
PART = 1 << 30  # 1 GiB (Scaleway: <= 1000 parts per upload -> objects up to ~1 TB)
EXPIRES = 7 * 24 * 3600
ROOT = Path(__file__).resolve().parent
STATE = ROOT / "state"
HOSTS = {k: os.environ[f"PLURIBUS_HOST_{k.upper()}"] for k in ("A", "B", "D", "dev") if f"PLURIBUS_HOST_{k.upper()}" in os.environ}  # e.g. PLURIBUS_HOST_A=203.0.113.10
KH = {"dev": "~/.ssh/known_hosts_pluribus_dev"}
REMOTE_DIR = "~/archive"


def s3():
    return boto3.client(
        "s3", endpoint_url=ENDPOINT, region_name=REGION,
        aws_access_key_id=os.environ["SCW_ACCESS_KEY"], aws_secret_access_key=os.environ["SCW_SECRET_KEY"],
        config=Config(signature_version="s3v4", s3={"addressing_style": "virtual"}),
    )


def ssh_args(box):
    kh = os.path.expanduser(KH.get(box, "~/.ssh/known_hosts_pluribus"))
    return ["-i", os.path.expanduser("~/.ssh/pluribus_ed25519"), "-o", "IdentitiesOnly=yes", "-o", f"UserKnownHostsFile={kh}"]


def ssh(box, cmd):
    return subprocess.run(["ssh", *ssh_args(box), f"ubuntu@{HOSTS[box]}", cmd], check=True, capture_output=True, text=True).stdout


def scp(box, local, remote):
    subprocess.run(["scp", "-q", *ssh_args(box), str(local), f"ubuntu@{HOSTS[box]}:{remote}"], check=True)


def state_path(name):
    STATE.mkdir(exist_ok=True)
    return STATE / f"{name}.json"


def cmd_plan(a):
    pairs = [p.split("=", 1) for p in a.files]
    srcs = [s for s, _ in pairs]
    out = ssh(a.box, "stat -L -c '%s %n' " + " ".join(f"'{s}'" for s in srcs))
    sizes = {}
    for line in out.strip().splitlines():
        sz, path = line.split(" ", 1)
        sizes[path] = int(sz)
    c = s3()
    jobs, lines = [], []
    for src, key in pairs:
        size = sizes[src]
        up = c.create_multipart_upload(Bucket=BUCKET, Key=key, StorageClass=a.cls)
        nparts = max(1, -(-size // PART))
        assert nparts <= 1000, f"{key}: {nparts} parts > 1000"
        for i in range(nparts):
            off, ln = i * PART, min(PART, size - i * PART)
            url = c.generate_presigned_url(
                "upload_part", ExpiresIn=EXPIRES,
                Params={"Bucket": BUCKET, "Key": key, "UploadId": up["UploadId"], "PartNumber": i + 1})
            lines.append("\t".join([key, str(i + 1), str(off), str(ln), src, url]))
        jobs.append({"src": src, "key": key, "size": size, "upload_id": up["UploadId"], "parts": nparts, "class": a.cls})
        print(f"planned {key}: {size/1e9:.2f} GB, {nparts} parts")
    st = {"box": a.box, "name": a.name, "created": time.time(), "jobs": jobs}
    state_path(a.name).write_text(json.dumps(st, indent=1))
    plan = STATE / f"{a.name}.plan.tsv"
    plan.write_text("\n".join(lines) + "\n")
    os.chmod(plan, 0o600)
    print(f"plan: {plan} ({len(lines)} parts)")


def cmd_start(a):
    ssh(a.box, f"mkdir -p {REMOTE_DIR}/{a.name} && chmod 700 {REMOTE_DIR}")
    scp(a.box, STATE / f"{a.name}.plan.tsv", f"{REMOTE_DIR}/{a.name}/plan.tsv")
    scp(a.box, ROOT / "upload_parts.sh", f"{REMOTE_DIR}/upload_parts.sh")
    # `cd` must not be in the same &&-list as the backgrounded command: `a && b &` forks a subshell
    # that keeps ssh's stdout open and the ssh call never returns.
    ssh(a.box, f"chmod 700 {REMOTE_DIR}/upload_parts.sh; cd {REMOTE_DIR}/{a.name} || exit 1; "
               f"setsid nohup {REMOTE_DIR}/upload_parts.sh plan.tsv {a.jobs} > run.log 2>&1 < /dev/null & echo started")
    print("started", a.box, a.name)


def done_parts(a):
    out = ssh(a.box, f"cat {REMOTE_DIR}/{a.name}/done.tsv 2>/dev/null || true")
    d = {}
    for line in out.strip().splitlines():
        key, part, etag = line.split("\t")
        d[(key, int(part))] = etag
    return d


def cmd_status(a):
    st = json.loads(state_path(a.name).read_text())
    d = done_parts(a)
    tot = sum(j["parts"] for j in st["jobs"])
    print(f"{a.name}: {len(d)}/{tot} parts uploaded")
    for j in st["jobs"]:
        n = sum(1 for (k, _) in d if k == j["key"])
        print(f"  {j['key']}: {n}/{j['parts']}")
    print(ssh(a.box, f"tail -3 {REMOTE_DIR}/{a.name}/run.log 2>/dev/null; cat {REMOTE_DIR}/{a.name}/sha256.txt 2>/dev/null; true"))


def cmd_complete(a):
    st = json.loads(state_path(a.name).read_text())
    d = done_parts(a)
    c = s3()
    sums = ssh(a.box, f"cat {REMOTE_DIR}/{a.name}/sha256.txt 2>/dev/null || true")
    ok = True
    for j in st["jobs"]:
        if j.get("completed"):
            continue
        parts = [{"PartNumber": p, "ETag": d[(j["key"], p)]} for p in range(1, j["parts"] + 1) if (j["key"], p) in d]
        if len(parts) != j["parts"]:
            print(f"  {j['key']}: only {len(parts)}/{j['parts']} parts — not completing"); ok = False; continue
        c.complete_multipart_upload(Bucket=BUCKET, Key=j["key"], UploadId=j["upload_id"], MultipartUpload={"Parts": parts})
        h = c.head_object(Bucket=BUCKET, Key=j["key"])
        good = h["ContentLength"] == j["size"]
        j["completed"] = good
        print(f"  {j['key']}: {h['ContentLength']} bytes {'OK' if good else 'SIZE MISMATCH'} ({h.get('StorageClass', 'STANDARD')})")
        ok &= good
    state_path(a.name).write_text(json.dumps(st, indent=1))
    if sums.strip():
        key = f"manifests/{a.name}.sha256"
        c.put_object(Bucket=BUCKET, Key=key, Body=sums.encode())
        print(f"  sha256 manifest -> {key}")
    sys.exit(0 if ok else 1)


def cmd_fetch(a):
    # Restore direction: presign GETs here, the box pulls them with parallel ranged requests (aria2c).
    c = s3()
    lines, jobs = [], []
    for key, dest in (p.split("=", 1) for p in a.files):
        h = c.head_object(Bucket=BUCKET, Key=key)
        if h.get("StorageClass") == "GLACIER" and 'ongoing-request="false"' not in (h.get("Restore") or ""):
            sys.exit(f"{key}: GLACIER and not restored yet ({h.get('Restore')})")
        url = c.generate_presigned_url("get_object", ExpiresIn=EXPIRES, Params={"Bucket": BUCKET, "Key": key})
        d = os.path.dirname(dest) or "."
        lines += [url, f"  dir={d}", f"  out={os.path.basename(dest)}"]
        jobs.append({"key": key, "dest": dest, "size": h["ContentLength"]})
        print(f"fetch {key} -> {dest} ({h['ContentLength']/1e9:.2f} GB)")
    st = {"box": a.box, "name": a.name, "created": time.time(), "fetch": jobs}
    state_path(a.name).write_text(json.dumps(st, indent=1))
    lst = STATE / f"{a.name}.fetch.txt"
    lst.write_text("\n".join(lines) + "\n")
    os.chmod(lst, 0o600)
    ssh(a.box, f"mkdir -p {REMOTE_DIR}/{a.name} && chmod 700 {REMOTE_DIR}; "
               "command -v aria2c >/dev/null || sudo apt-get -qq install -y aria2 >/dev/null 2>&1; "
               + "; ".join(f"mkdir -p '{os.path.dirname(j['dest']) or '.'}'" for j in jobs))
    scp(a.box, lst, f"{REMOTE_DIR}/{a.name}/fetch.txt")
    ssh(a.box, f"cd {REMOTE_DIR}/{a.name} || exit 1; setsid nohup sh -c 'aria2c -i fetch.txt -j 4 -x 16 -s 16 -k 64M "
               f"--file-allocation=falloc --auto-file-renaming=false --allow-overwrite=true --max-tries=20 --retry-wait=10 "
               f"--console-log-level=warn --summary-interval=60 && echo FETCH-DONE || echo FETCH-FAILED' > run.log 2>&1 < /dev/null & echo started")
    print("started", a.box, a.name)


def cmd_fetched(a):
    st = json.loads(state_path(a.name).read_text())
    out = ssh(a.box, f"tail -2 {REMOTE_DIR}/{a.name}/run.log; " + "; ".join(f"stat -c '%s %n' '{j['dest']}' 2>/dev/null" for j in st["fetch"]) + "; true")
    print(out)
    sizes = {}
    for line in out.strip().splitlines():
        sz, _, path = line.partition(" ")
        if sz.isdigit():
            sizes[path] = int(sz)
    ok = "FETCH-DONE" in out and all(sizes.get(j["dest"]) == j["size"] for j in st["fetch"])
    print("all sizes match" if ok else "NOT complete / size mismatch")
    sys.exit(0 if ok else 1)


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd", required=True)
    for name in ("fetch", "fetched"):
        q = sub.add_parser(name)
        q.add_argument("--box", required=True, choices=list(HOSTS))
        q.add_argument("--name", required=True)
        if name == "fetch":
            q.add_argument("files", nargs="+", help="KEY=DEST (DEST is an absolute path on the box)")
    for name in ("plan", "start", "status", "complete"):
        q = sub.add_parser(name)
        q.add_argument("--box", required=True, choices=list(HOSTS))
        q.add_argument("--name", required=True)
        if name == "plan":
            q.add_argument("--class", dest="cls", default="STANDARD", choices=["STANDARD", "ONEZONE_IA", "GLACIER"])
            q.add_argument("files", nargs="+", help="SRC=KEY (SRC is a path on the box)")
        if name == "start":
            q.add_argument("--jobs", type=int, default=6)
    a = p.parse_args()
    {"plan": cmd_plan, "start": cmd_start, "status": cmd_status, "complete": cmd_complete,
     "fetch": cmd_fetch, "fetched": cmd_fetched}[a.cmd](a)


if __name__ == "__main__":
    main()
