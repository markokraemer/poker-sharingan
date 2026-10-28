"""Run a benchmark-track run for a house bot (server-hosted, verified) and print progress + throughput.
   python3 house_run.py http://127.0.0.1:8080 ADMIN_TOKEN ladder-v1 house:tight_aggressive [hands]"""
import json, sys, time, urllib.request

base, admin, track, bot = sys.argv[1:5]
hands = int(sys.argv[5]) if len(sys.argv) > 5 else None


def call(method, path, body=None):
    req = urllib.request.Request(base + path, data=json.dumps(body).encode() if body is not None else None, method=method,
                                 headers={"authorization": f"Bearer {admin}", "content-type": "application/json"})
    with urllib.request.urlopen(req) as r:
        return json.loads(r.read())


t0 = time.time()
run = call("POST", f"/v1/benchmarks/{track}/runs", {"player_id": bot, **({"hands": hands} if hands else {})})
print("started", run["run_id"], "planned", run["hands_planned"], "verified", run["verified"])
last = 0
while True:
    r = call("GET", f"/v1/benchmarks/{track}/runs/{run['run_id']}")
    if time.time() - last > 2 or r["status"] != "running":
        print(f"  {r['status']} {r['hands_done']}/{r['hands_total']} running={r.get('running_mbb_per_game')} +-{r.get('running_ci95_mbb')}")
        last = time.time()
    if r["status"] != "running":
        break
    time.sleep(0.2)
dt = time.time() - t0
print(f"{r['hands_done']} hands in {dt:.1f}s = {r['hands_done']/dt:.0f} hands/s; score {r['mbb_per_game']:+.1f} +- {r['ci95_mbb']:.1f} mbb/game ({r['method']}) verified={r['verified']}")
print(json.dumps(call("GET", "/v1/leaderboard?track=" + track))["rows"][:1] if False else "")
