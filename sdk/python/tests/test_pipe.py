"""Pipe protocol (docs/TABLE_API.md section 14): `sharingan pipe`, the PipeRunner and `eval --local`."""
import io
import json
import os
import subprocess
import sys
import textwrap

import pytest

from sharingan import Action, Bot, cli
from sharingan.pipe import PipeRunner

from conftest import DOC_YOUR_TURN

HERE = os.path.dirname(os.path.abspath(__file__))
PKG_PARENT = os.path.dirname(HERE)


def turn_env(**over):
    e = json.loads(json.dumps(DOC_YOUR_TURN))
    e["data"]["hand_id"] = "h-77"
    e.update(over)
    return e


def lines(*envs):
    return io.BytesIO(b"".join((json.dumps(e) + "\n").encode() for e in envs))


def run_runner(bot, *envs):
    out = io.BytesIO()
    rc = PipeRunner(bot, lines(*envs), out).run()
    return rc, [json.loads(l) for l in out.getvalue().decode().splitlines()]


class Raiser(Bot):
    name = "raiser"

    def act(self, state):
        return Action.raise_to(400)


def test_handshake_act_and_bye():
    welcome = {"v": 1, "type": "welcome", "data": {"protocol": 1, "mode": "benchmark", "tables": ["t1"]}}
    rc, out = run_runner(Raiser(), welcome, turn_env(), {"v": 1, "type": "bye", "data": {}})
    assert rc == 0
    assert [m["type"] for m in out] == ["hello", "act", "bye"]
    assert out[0]["data"]["protocol"] == 1 and out[0]["data"]["bot"] == "raiser"
    act = out[1]
    assert act["table_id"] == "t1"
    assert act["data"] == {"hand_id": "h-77", "decision_id": "d-77-9", "type": "raise", "to": 400}


def test_illegal_answer_is_legalized_and_exception_plays_check_fold():
    class Wild(Bot):
        def act(self, state):
            return Action.raise_to(1)  # below min_to=100 -> legalize clamps it

    _, out = run_runner(Wild(), turn_env())
    assert out[1]["data"]["type"] == "raise" and out[1]["data"]["to"] == 100

    class Boom(Bot):
        def act(self, state):
            raise RuntimeError("bug in my bot")

    _, out = run_runner(Boom(), turn_env())
    assert out[1]["data"]["type"] == "check"  # check is free in the doc example, else fold

    class NotAnAction(Bot):
        def act(self, state):
            return "call"

    _, out = run_runner(NotAnAction(), turn_env())
    assert out[1]["data"]["type"] == "check"


def test_async_act_and_hooks_only_when_overridden():
    seen = []

    class A(Bot):
        async def act(self, state):
            return Action.call()

        def on_hand_end(self, end):
            seen.append(end.hand_id)

        def on_connect(self, welcome):
            seen.append(welcome["protocol"])

    hand_end = {"v": 1, "type": "hand_end", "data": {"hand_id": "h-77", "results": [], "hole_cards": {"0": ["Ah", "Kd"]}}}
    _, out = run_runner(A(), {"v": 1, "type": "welcome", "data": {"protocol": 1}}, turn_env(), hand_end)
    assert seen == [1, "h-77"]
    assert [m["type"] for m in out] == ["hello", "act"]  # hand_end gets no answer

    # a bot without on_hand_end: the payload is not even parsed (garbage-tolerant, fast)
    _, out = run_runner(Raiser(), {"v": 1, "type": "hand_end", "data": "ignored"})
    assert [m["type"] for m in out] == ["hello"]


def test_garbage_lines_are_skipped():
    out = io.BytesIO()
    src = io.BytesIO(b"not json\n\n" + (json.dumps(turn_env()) + "\n").encode())
    PipeRunner(Raiser(), src, out).run()
    assert [json.loads(l)["type"] for l in out.getvalue().decode().splitlines()] == ["hello", "act"]


def test_subprocess_keeps_stdout_clean(tmp_path):
    """`sharingan pipe` over real pipes: prints in the bot (import time and inside act) must not reach the protocol stream."""
    bot = tmp_path / "chatty.py"
    bot.write_text(textwrap.dedent("""
        print("import-time noise")
        from sharingan import Bot, Action
        class Chatty(Bot):
            def act(self, state):
                print("act-time noise")
                return Action.call() if state.legal.call else Action.check()
    """))
    env = dict(os.environ, PYTHONPATH=PKG_PARENT)
    p = subprocess.Popen([sys.executable, "-m", "sharingan.cli", "pipe", str(bot)], stdin=subprocess.PIPE,
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
    hello = json.loads(p.stdout.readline())
    assert hello["type"] == "hello"
    p.stdin.write((json.dumps({"v": 1, "type": "welcome", "data": {"protocol": 1}}) + "\n").encode())
    p.stdin.write((json.dumps(turn_env()) + "\n").encode())
    p.stdin.flush()
    act = json.loads(p.stdout.readline())
    assert act["type"] == "act" and act["data"]["type"] == "check"
    p.stdin.write((json.dumps({"v": 1, "type": "bye", "data": {}}) + "\n").encode())
    p.stdin.flush()
    assert json.loads(p.stdout.readline())["type"] == "bye"
    rest, err = p.communicate(timeout=10)
    assert rest == b"" and p.returncode == 0
    assert b"import-time noise" in err and b"act-time noise" in err


def test_eval_local_without_worker_prints_fallback(tmp_path, monkeypatch, capsys):
    monkeypatch.delenv("SHARINGAN_ARENA_BIN", raising=False)
    monkeypatch.setenv("PATH", str(tmp_path))  # no sharingan-arena here
    bot = tmp_path / "b.py"
    bot.write_text("from sharingan import Bot, Action\nclass B(Bot):\n    def act(self, s):\n        return Action.check()\n")
    rc = cli.main(["eval", str(bot), "--local"])
    err = capsys.readouterr().err
    assert rc == 2
    assert "sharingan-arena" in err and "PATH" in err and "cargo build --release -p sharingan-arena" in err


def test_eval_local_runs_the_worker_binary(tmp_path, monkeypatch, capsys):
    """A fake worker: checks the command line the SDK builds and writes a result.json."""
    fake = tmp_path / "sharingan-arena"
    fake.write_text(textwrap.dedent(f"""\
        #!{sys.executable}
        import json, sys
        a = sys.argv[1:]
        assert a[0] == "run", a
        get = lambda k: a[a.index(k) + 1]
        assert "sharingan.cli pipe" in get("--bot") and get("--bot").endswith("b.py'") or get("--bot").endswith("b.py")
        json.dump({{"track": get("--track"), "hands": int(get("--hands")), "mbb_per_game": 12.5, "ci95_mbb": 3.0,
                   "method": "aivat", "forfeit_hands": 0, "timings": {{"hands_per_sec": 1234.0}}}}, open(get("--out"), "w"))
    """))
    fake.chmod(0o755)
    monkeypatch.setenv("SHARINGAN_ARENA_BIN", str(fake))
    bot = tmp_path / "b.py"
    bot.write_text("from sharingan import Bot, Action\nclass B(Bot):\n    def act(self, s):\n        return Action.check()\n")
    rc = cli.main(["eval", str(bot), "--local", "--track", "sharingan-1", "--hands", "600", "--lanes", "4"])
    out = capsys.readouterr().out
    assert rc == 0
    assert "+12.5 ± 3.0 mbb/game" in out and "600 hands on sharingan-1" in out and "local" in out


@pytest.mark.skipif(not (cli.find_worker() and os.environ.get("ARENA_BLUEPRINT")), reason="needs sharingan-arena on PATH and ARENA_BLUEPRINT")
def test_eval_local_end_to_end_with_the_real_worker(tmp_path, capsys):
    bot = tmp_path / "station.py"
    bot.write_text("from sharingan import Bot\nfrom sharingan.helpers import call_or_check\nclass S(Bot):\n    def act(self, s):\n        return call_or_check(s)\n")
    out = tmp_path / "r.json"
    rc = cli.main(["eval", str(bot), "--local", "--hands", "600", "--lanes", "2", "--out", str(out), "--quiet"])
    assert rc == 0
    r = json.loads(out.read_text())
    assert r["hands"] == 600 and r["forfeit_hands"] == 0 and r["method"] == "aivat"
    assert "mbb/game" in capsys.readouterr().out
