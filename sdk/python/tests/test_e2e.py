"""End-to-end against a real arena server. Skipped unless SHARINGAN_E2E_SERVER is set, e.g.
   SHARINGAN_E2E_SERVER=http://localhost:8095 pytest tests/test_e2e.py
(server needs ARENA_BLUEPRINT so the benchmark tracks are available)."""
import json
import os
import re
import signal
import subprocess
import sys
import time
import urllib.request

import pytest

SERVER = os.environ.get("SHARINGAN_E2E_SERVER")
pytestmark = pytest.mark.skipif(not SERVER, reason="set SHARINGAN_E2E_SERVER to run against a real arena")

BOT = "from sharingan.bots import CallingStation\nbot = CallingStation()\n"


def _cli(*args, **kw):
    return subprocess.run([sys.executable, "-m", "sharingan.cli", *args], capture_output=True, text=True, timeout=300, **kw)


def test_benchmark_and_eval(tmp_path):
    bot = tmp_path / "e2ebot.py"
    bot.write_text(BOT)
    r = _cli("benchmark", str(bot), "--track", "ladder-v1", "--hands", "300", "--server", SERVER)
    assert r.returncode == 0, r.stderr
    out = r.stdout.strip().splitlines()
    assert re.fullmatch(r"calling_station: [+-]\d+\.\d ± \d+\.\d mbb/game \(95% CI, aivat\) over 300 hands on ladder-v1 \[unverified\]", out[0]), out
    assert out[1] == f"leaderboard: {SERVER.rstrip('/')}/v1/leaderboard?track=ladder-v1"
    assert "ladder-v1: 100% 300/300 hands" in r.stderr
    e = _cli("eval", str(bot), "--hands", "300", "--server", SERVER, "--quiet")
    assert e.returncode == 0 and "on ladder-v1" in e.stdout and len(e.stdout.strip().splitlines()) == 1, e.stderr


def test_connect_then_external_run(tmp_path):
    bot = tmp_path / "e2ebot.py"
    bot.write_text(BOT)
    req = urllib.request.Request(SERVER + "/v1/players", data=json.dumps({"name": "e2e-connect", "kind": "bot"}).encode(),
                                 method="POST", headers={"content-type": "application/json"})
    reg = json.load(urllib.request.urlopen(req))
    p = subprocess.Popen([sys.executable, "-m", "sharingan.cli", "connect", str(bot), "--server", SERVER, "--token", reg["token"],
                          "--player-id", reg["player_id"]], stderr=subprocess.PIPE, text=True)
    try:
        run, t0 = None, time.time()
        while run is None and time.time() - t0 < 20:
            try:
                req = urllib.request.Request(SERVER + "/v1/benchmarks/ladder-v1/runs", data=json.dumps({"hands": 120}).encode(), method="POST",
                                             headers={"content-type": "application/json", "authorization": "Bearer " + reg["token"]})
                run = json.load(urllib.request.urlopen(req))
            except urllib.error.HTTPError as e:
                assert e.code == 409  # not_connected yet
                time.sleep(0.3)
        assert run, "run never started"
        while time.time() - t0 < 120:
            cur = json.load(urllib.request.urlopen(f"{SERVER}/v1/benchmarks/ladder-v1/runs/{run['run_id']}"))
            if cur["status"] != "running":
                break
            time.sleep(0.3)
        assert cur["status"] == "finished" and cur["mbb_per_game"] is not None
    finally:
        p.send_signal(signal.SIGTERM)
        err = p.communicate(timeout=20)[1]
    assert p.returncode == 0 and f"connected as {reg['player_id']}" in err and "disconnected after 120 hands" in err
