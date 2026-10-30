import os
import asyncio
import subprocess
import sys

import pytest

from sharingan.cli import FunctionBot, load_bot, main, format_score
from sharingan.mock import MockArena
from sharingan import Bot


def test_bot_token_is_saved_and_reused(tmp_path):
    from sharingan import cli

    class FakeApi:
        base_url = "http://arena.test"
        registered = 0

        def register_bot(self, name):
            FakeApi.registered += 1
            from types import SimpleNamespace
            return SimpleNamespace(player_id=f"p_{FakeApi.registered}", token=f"sk_{FakeApi.registered}")

        def request(self, method, path, body=None, token=None, query=None):
            if token != "sk_1":
                raise cli.ApiException(401, "unauthorized", "bad token")
            return {"player_id": "p_1"}

    api = FakeApi()
    assert cli.bot_token(api, "mybot") == "sk_1"
    assert cli.bot_token(api, "mybot") == "sk_1", "second run reuses the saved token"
    assert FakeApi.registered == 1
    assert oct(os.stat(cli.TOKENS_FILE).st_mode & 0o777) == "0o600"
    assert cli.bot_token(api, "other") == "sk_2" and FakeApi.registered == 2, "another bot name is another bot"
    assert cli.bot_token(api, "other") == "sk_3" and FakeApi.registered == 3, "a rejected saved token is replaced"
    assert cli.leaderboard_url("https://arena.sharingan.poker", "ladder-v1") == "https://sharingan.poker/leaderboard/"


def test_load_bot_variants(tmp_path):
    a = tmp_path / "a.py"
    a.write_text("from sharingan import Bot, Action\nclass Mine(Bot):\n    def act(self, s):\n        return Action.check()\n")
    b = tmp_path / "b.py"
    b.write_text("def act(state):\n    return None\n")
    c = tmp_path / "c.py"
    c.write_text("from sharingan.bots import CallingStation\nbot = CallingStation()\n")
    d = tmp_path / "d.py"
    d.write_text("x = 1\n")
    assert isinstance(load_bot(str(a)), Bot) and load_bot(str(a)).name == "a"
    assert isinstance(load_bot(str(b)), FunctionBot)
    assert load_bot(str(c)).name == "calling_station"
    with pytest.raises(SystemExit):
        load_bot(str(d))


def test_format_score():
    line = format_score("x", {"bb_per_100": 5.0, "ci95_bb_per_100": 2.5}, 600, "ladder")
    assert line == "x: +50.0 ± 25.0 mbb/game (95% CI) over 600 hands vs ladder"
    assert "[duplicate]" in format_score("x", {"duplicate_bb_per_100": 1.0}, 6, "ladder")


import contextlib
import threading


@contextlib.contextmanager
def mock_server(**kw):
    """MockArena on a background loop thread (main() runs its own asyncio.run)."""
    ready, stop, box = threading.Event(), threading.Event(), {}

    def serve():
        async def run():
            async with MockArena(**kw) as arena:
                box["arena"] = arena
                ready.set()
                while not stop.is_set():
                    await asyncio.sleep(0.05)
        asyncio.run(run())
    t = threading.Thread(target=serve, daemon=True)
    t.start()
    assert ready.wait(10)
    try:
        yield box["arena"]
    finally:
        stop.set()
        t.join(5)


def _bot(tmp_path):
    bot = tmp_path / "mybot.py"
    bot.write_text("def act(s):\n    return None\n")
    return str(bot)


def test_eval_prints_one_line(tmp_path, capsys):
    bot = tmp_path / "mybot.py"
    bot.write_text("def act(s):\n    return None\n")
    with mock_server() as arena:  # no /v1/benchmarks -> falls back to the duplicate match
        rc = main(["eval", str(bot), "--server", arena.url, "--hands", "600"])
    out = capsys.readouterr().out.strip().splitlines()
    assert rc == 0 and out == ["mybot: +40.0 ± 25.0 mbb/game (95% CI) over 600 hands vs ladder [duplicate]"]


def test_unreachable_prints_start_command(tmp_path, capsys):
    bot = tmp_path / "mybot.py"
    bot.write_text("def act(s):\n    return None\n")
    rc = main(["eval", str(bot), "--server", "http://127.0.0.1:1"])
    assert rc == 2 and "cargo run --release -p sharingan-arena" in capsys.readouterr().err


def test_eval_defaults_to_ladder_benchmark(tmp_path, capsys):
    with mock_server(benchmarks=True) as arena:
        rc = main(["eval", _bot(tmp_path), "--server", arena.url, "--hands", "600"])
        assert arena.run_requests == [{"hands": 600}]
    out = capsys.readouterr().out.strip().splitlines()
    assert rc == 0 and out == ["mybot: +42.5 ± 12.0 mbb/game (95% CI, aivat) over 600 hands on ladder-v1 [unverified]"]


def test_eval_match_flag_forces_duplicate_match(tmp_path, capsys):
    with mock_server(benchmarks=True) as arena:
        rc = main(["eval", _bot(tmp_path), "--server", arena.url, "--hands", "600", "--match"])
        assert arena.run_requests == []
    assert rc == 0 and "[duplicate]" in capsys.readouterr().out


def test_benchmark_prints_score_and_leaderboard(tmp_path, capsys):
    with mock_server(benchmarks=True, bench_result=(-7.25, 3.0)) as arena:
        rc = main(["benchmark", _bot(tmp_path), "--track", "sharingan-1", "--hands", "100", "--server", arena.url])
        assert arena.run_requests == [{"hands": 100}] and arena.runs
        run = next(iter(arena.runs.values()))
        assert run["track"] == "sharingan-1" and run["hands_total"] == 102  # rounded up to whole deals
    cap = capsys.readouterr()
    assert rc == 0
    assert cap.out.strip().splitlines() == [
        "mybot: -7.2 ± 3.0 mbb/game (95% CI, aivat) over 102 hands on sharingan-1 [unverified]",
        f"leaderboard: {arena.url}/v1/leaderboard?track=sharingan-1"]
    assert "sharingan-1: 100% 102/102 hands" in cap.err  # progress lines (stderr is not a tty here)


def test_benchmark_reuses_token_and_quiet(tmp_path, capsys):
    import json
    import urllib.request
    with mock_server(benchmarks=True) as arena:
        req = urllib.request.Request(arena.url + "/v1/players", data=json.dumps({"name": "pre"}).encode(), method="POST")
        reg = json.loads(urllib.request.urlopen(req).read())
        rc = main(["benchmark", _bot(tmp_path), "--token", reg["token"], "--quiet", "--server", arena.url])
        assert len(arena.players) == 1  # no second registration
        assert next(iter(arena.runs.values()))["player_id"] == reg["player_id"]
        assert arena.run_requests == [{}]  # server default hands
    cap = capsys.readouterr()
    assert rc == 0 and "hands on ladder-v1" in cap.out and cap.err == ""


def test_benchmark_unreachable(tmp_path, capsys):
    rc = main(["benchmark", _bot(tmp_path), "--server", "http://127.0.0.1:1"])
    assert rc == 2 and "arena server not reachable" in capsys.readouterr().err


def test_connect_answers_until_max_hands(tmp_path, capsys):
    import json
    import urllib.request
    with mock_server(hands=5) as arena:
        req = urllib.request.Request(arena.url + "/v1/players", data=json.dumps({"name": "pre"}).encode(), method="POST")
        reg = json.loads(urllib.request.urlopen(req).read())
        rc = main(["connect", _bot(tmp_path), "--server", arena.url, "--token", reg["token"],
                   "--player-id", reg["player_id"], "--max-hands", "3"])
        assert rc == 0 and len(arena.hand_ends) == 3 and len(arena.acts) >= 3
    err = capsys.readouterr().err
    assert f"connected as {reg['player_id']}" in err and "warning" not in err and "disconnected after 3 hands" in err


def test_connect_warns_on_wrong_player_id_and_bad_token(tmp_path, capsys):
    with mock_server(hands=1) as arena:
        import json
        import urllib.request
        req = urllib.request.Request(arena.url + "/v1/players", data=json.dumps({"name": "pre"}).encode(), method="POST")
        reg = json.loads(urllib.request.urlopen(req).read())
        assert main(["connect", _bot(tmp_path), "--server", arena.url, "--token", reg["token"],
                     "--player-id", "someone-else", "--max-hands", "1"]) == 0
        assert "warning: token belongs to" in capsys.readouterr().err
        assert main(["connect", _bot(tmp_path), "--server", arena.url, "--token", "nope", "--no-reconnect"]) == 1
        assert "never connected" in capsys.readouterr().err


def test_launch_bot_wrapper_maps_env(tmp_path, monkeypatch):
    """deploy/platinum/bot/launch_bot.py is a thin wrapper over `sharingan connect`."""
    import importlib.util
    import os
    path = os.path.join(os.path.dirname(__file__), "..", "..", "..", "deploy", "platinum", "bot", "launch_bot.py")
    spec = importlib.util.spec_from_file_location("launch_bot", path)
    lb = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(lb)
    monkeypatch.setenv("BOT_FILE", "/x/bot.py")
    monkeypatch.setenv("ARENA_URL", "https://a")
    monkeypatch.setenv("ARENA_TOKEN", "T")
    monkeypatch.setenv("MAX_HANDS", "7")
    assert lb.argv_from_env() == ["connect", "/x/bot.py", "--server", "https://a", "--token", "T", "--max-hands", "7"]


def test_submit_sends_contact_only_when_given(tmp_path, monkeypatch, capsys):
    from sharingan import cli
    sent = []

    class FakeApi:
        def __init__(self, base_url):
            self.base_url, self.timeout = base_url, 30.0

        def request(self, method, path, body=None, token=None, query=None):
            if method == "POST":
                sent.append(body)
                return {"id": "job_1"}
            return {"status": "finished", "settled": True, "hands_done": 600, "hands_total": 600, "bot_name": "mybot",
                    "mbb_per_game": 1.0, "ci95_mbb": 2.0, "track": "ladder-v1", "verified": True}

    monkeypatch.setattr(cli, "ArenaClient", FakeApi)
    monkeypatch.delenv("SHARINGAN_CONTACT", raising=False)
    bot = _bot(tmp_path)
    assert main(["submit", bot, "--token", "sk_x", "--server", "http://x", "--contact", "@me", "--quiet"]) == 0
    assert main(["submit", bot, "--token", "sk_x", "--server", "http://x", "--quiet"]) == 0
    assert sent[0]["contact"] == "@me" and "contact" not in sent[1]
    assert "[VERIFIED, hosted job job_1]" in capsys.readouterr().out


def test_submit_warns_when_the_bot_crashed_or_forfeited(tmp_path, monkeypatch, capsys):
    from sharingan import cli

    class FakeApi:
        def __init__(self, base_url):
            self.base_url, self.timeout = base_url, 30.0

        def request(self, method, path, body=None, token=None, query=None):
            if method == "POST":
                return {"id": "job_2"}
            return {"status": "finished", "settled": True, "hands_done": 600, "hands_total": 600, "bot_name": "mybot",
                    "mbb_per_game": -208.3, "ci95_mbb": 205.7, "track": "sharingan-1", "verified": True, "forfeit_hands": 588,
                    "bot_log_tail": "lane 1: RuntimeError: boom\nplaying check/fold"}

    monkeypatch.setattr(cli, "ArenaClient", FakeApi)
    monkeypatch.delenv("SHARINGAN_CONTACT", raising=False)
    assert main(["submit", _bot(tmp_path), "--token", "sk_x", "--server", "http://x", "--track", "sharingan-1", "--quiet"]) == 0
    err = capsys.readouterr().err
    assert "588 of 600 hands forfeited" in err and "RuntimeError: boom" in err
    assert "--contact" in err, "a prize-track submit without a contact is nudged"
