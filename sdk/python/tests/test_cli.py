import os
import asyncio
import subprocess
import sys
from typing import Any

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


def test_submit_requires_contact_and_profile_and_remembers_them(tmp_path, monkeypatch, capsys):
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
    bot = _bot(tmp_path)
    base = ["submit", bot, "--token", "sk_x", "--server", "http://x", "--quiet"]
    assert main(base) == 2 and not sent, "nothing is uploaded without an owner"
    assert "--contact" in capsys.readouterr().err
    assert main(base + ["--contact", "me@example.com"]) == 2 and not sent, "an email alone is not enough"
    assert main(base + ["--contact", "@me", "--profile", "https://x.com/me"]) == 2 and not sent, "contact is an email"
    assert main(base + ["--contact", "me@example.com", "--profile", "https://x.com/me"]) == 0
    assert main(base) == 0, "remembered after the first accepted submit"
    assert [(b["contact"], b["profile"]) for b in sent] == [("me@example.com", "https://x.com/me")] * 2
    assert oct(os.stat(cli.OWNER_FILE).st_mode & 0o777) == "0o600"
    out = capsys.readouterr()
    assert "[VERIFIED, NOT RANKED: under 20,004 hands, hosted job job_1]" in out.out
    assert "leaderboard:" not in out.out, "a run that cannot rank does not link the board"
    assert "not ranked" in out.err, "a verified run shorter than a ranked run says it does not count"


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
    monkeypatch.setenv("SHARINGAN_CONTACT", "me@example.com")
    monkeypatch.setenv("SHARINGAN_PROFILE", "https://linkedin.com/in/me")
    assert main(["submit", _bot(tmp_path), "--token", "sk_x", "--server", "http://x", "--track", "sharingan-1", "--quiet"]) == 0
    err = capsys.readouterr().err
    assert "588 of 600 hands forfeited" in err and "RuntimeError: boom" in err


def test_the_warning_names_the_exception_not_the_traceback_header():
    from sharingan import cli
    import io, contextlib
    tail = "Traceback (most recent call last):\n  File \"/opt/sharingan/bot/bot.py\", line 9, in act\n    raise ValueError(\"boom on the river\")\nValueError: boom on the river"
    err = io.StringIO()
    with contextlib.redirect_stderr(err):
        cli._warn_bot_trouble({"id": "job_9", "hands_done": 20004, "forfeit_hands": 0, "bot_log_tail": tail})
    out = err.getvalue()
    assert "first error: ValueError: boom on the river" in out, out
    assert "bot_log_tail" in out and "job_9" in out


# ----------------------------------------------------------------------------- token / submit / play / naming


class FakeApi:
    """Stands in for ArenaClient: no network. `registered` counts POST /v1/players; `posted` keeps the /v1/jobs bodies."""
    registered: list = []
    posted: list = []
    job: dict = {}
    refuse: Any = None

    def __init__(self, base_url, token=None):
        self.base_url, self.token, self.timeout = base_url.rstrip("/"), token, 30.0

    def register_bot(self, name, kind="bot", set_token=True):
        from types import SimpleNamespace
        FakeApi.registered.append(name)
        n = len(FakeApi.registered)
        self.token = f"sk_{n}"
        return SimpleNamespace(player_id=f"p_{n}", token=f"sk_{n}")

    def request(self, method, path, body=None, token=None, query=None):
        if path == "/v1/me":
            return {"player_id": "p_" + (token or "")[3:]}
        if method == "POST":
            if FakeApi.refuse:
                raise FakeApi.refuse
            FakeApi.posted.append(body)
            return {"id": "job_9", "hands": 20004}
        return dict({"status": "finished", "settled": True, "hands_done": 20004, "hands_total": 20004, "bot_name": "mybot",
                     "mbb_per_game": 12.5, "ci95_mbb": 3.0, "track": "sharingan-1", "verified": True}, **FakeApi.job)


@pytest.fixture
def fake_api(monkeypatch):
    from sharingan import cli
    FakeApi.registered, FakeApi.posted, FakeApi.job, FakeApi.refuse = [], [], {}, None
    monkeypatch.setattr(cli, "ArenaClient", FakeApi)
    monkeypatch.setenv("SHARINGAN_CONTACT", "me@example.com")
    monkeypatch.setenv("SHARINGAN_PROFILE", "https://x.com/me")
    monkeypatch.delenv("SHARINGAN_SERVER", raising=False)
    return FakeApi


def _save_token(server, name, token, player_id="p"):
    import json
    from sharingan import cli
    tokens = cli._load_tokens()
    tokens[f"{server}|{name}"] = {"player_id": player_id, "token": token}
    with open(cli.TOKENS_FILE, "w") as f:
        json.dump(tokens, f)


def test_token_prints_only_the_saved_token(tmp_path, capsys):
    _save_token("http://a.test", "mybot", "sk_mine")
    _save_token("http://a.test", "other", "sk_other")
    _save_token("http://b.test", "solo", "sk_solo")
    capsys.readouterr()
    assert main(["token", str(tmp_path / "mybot.py"), "--server", "http://a.test/"]) == 0, "a path is named after its file stem"
    assert capsys.readouterr().out == "sk_mine\n"
    assert main(["token", "other", "--server", "http://a.test"]) == 0, "or give the bot name"
    assert capsys.readouterr().out == "sk_other\n"
    assert main(["token", "--server", "http://b.test"]) == 0, "no BOT: the only token saved for that server"
    cap = capsys.readouterr()
    assert cap.out == "sk_solo\n" and cap.err == ""


def test_token_failures_are_exit_1_with_a_message_on_stderr(tmp_path, capsys):
    _save_token("http://a.test", "mybot", "sk_mine")
    _save_token("http://a.test", "other", "sk_other")
    capsys.readouterr()
    assert main(["token", "--server", "http://a.test"]) == 1, "two bots, none named"
    cap = capsys.readouterr()
    assert cap.out == "" and "mybot" in cap.err and "other" in cap.err
    assert main(["token", "nobody.py", "--server", "http://a.test"]) == 1
    cap = capsys.readouterr()
    assert cap.out == "" and "no saved token for bot 'nobody'" in cap.err
    assert main(["token", "--server", "http://empty.test"]) == 1
    cap = capsys.readouterr()
    assert cap.out == "" and "no saved token" in cap.err and "sharingan submit" in cap.err


def test_token_uses_the_default_server(monkeypatch, capsys):
    from sharingan import cli
    monkeypatch.delenv("SHARINGAN_SERVER", raising=False)
    _save_token(cli.DEFAULT_SERVER, "mybot", "sk_public")
    assert main(["token", "mybot.py"]) == 0 and capsys.readouterr().out == "sk_public\n"


@pytest.mark.parametrize("extra, why", [
    (["--hands", "599"], "--hands"),
    (["--hands", "200001"], "--hands"),
    (["--hands", "0"], "--hands"),
    (["--profile", "not a link"], "--profile"),
    (["--profile", "https://127.0.0.1/me"], "--profile"),
    (["--contact", "@me"], "--contact"),
])
def test_submit_validates_before_registering_or_uploading(tmp_path, fake_api, capsys, extra, why):
    assert main(["submit", _bot(tmp_path), "--server", "http://x", "--quiet"] + extra) == 2
    assert why in capsys.readouterr().err
    assert fake_api.registered == [] and fake_api.posted == [], "nothing was registered or uploaded"


def test_submit_accepts_the_edges_of_the_hands_range_and_handles(tmp_path, fake_api, capsys):
    for extra in (["--hands", "600"], ["--hands", "200000"], ["--profile", "@me"], ["--profile", "linkedin.com/in/me"]):
        assert main(["submit", _bot(tmp_path), "--server", "http://x", "--quiet", "--token", "sk_x"] + extra) == 0
    assert len(fake_api.posted) == 4


def test_submit_missing_entry_in_a_directory_does_not_register(tmp_path, fake_api, capsys):
    d = tmp_path / "botdir"
    d.mkdir()
    (d / "main.py").write_text("def act(s):\n    return None\n")
    assert main(["submit", str(d), "--server", "http://x", "--quiet"]) == 2
    assert "bot.py" in capsys.readouterr().err and fake_api.registered == [] and fake_api.posted == []
    assert main(["submit", str(d), "--entry", "main.py", "--server", "http://x", "--quiet"]) == 0
    assert fake_api.registered == ["botdir"] and fake_api.posted[0]["entry"] == "main.py"


def test_submit_accepted_line_shows_server_hands_and_upload_hash(tmp_path, fake_api, capsys):
    import hashlib
    assert main(["submit", _bot(tmp_path), "--server", "http://x", "--quiet", "--token", "sk_x"]) == 0
    cap = capsys.readouterr()
    accepted = [ln for ln in cap.err.splitlines() if "accepted" in ln]
    assert len(accepted) == 1 and "job job_9 accepted (20004 hands on ladder-v1; upload sha256 " in accepted[0], accepted
    import base64
    sha = hashlib.sha256(base64.b64decode(fake_api.posted[0]["artifactTgzBase64"])).hexdigest()
    assert f"sha256 {sha[:12]})" in accepted[0], "the hash of the uploaded .tar.gz"
    assert "[VERIFIED, hosted job job_9]" in cap.out and "leaderboard:" in cap.out, "a full verified run links the board"
    assert "not ranked" not in cap.err


def test_submit_short_verified_run_says_not_ranked_and_skips_the_link(tmp_path, fake_api, capsys):
    fake_api.job = {"hands_done": 600, "hands_total": 600, "mbb_per_game": 378.1, "ci95_mbb": 578.5}
    assert main(["submit", _bot(tmp_path), "--server", "http://x", "--quiet", "--token", "sk_x", "--hands", "600"]) == 0
    cap = capsys.readouterr()
    assert cap.out.strip().splitlines() == [
        "mybot: +378.1 +- 578.5 mbb/game over 600 hands on sharingan-1 [VERIFIED, NOT RANKED: under 20,004 hands, hosted job job_9]"]
    assert "not ranked" in cap.err


def test_submit_unverified_job_is_labelled_and_a_closed_track_is_explained(tmp_path, fake_api, capsys):
    fake_api.job = {"verified": False}
    assert main(["submit", _bot(tmp_path), "--server", "http://x", "--quiet", "--token", "sk_x"]) == 0
    assert "[unverified, hosted job job_9]" in capsys.readouterr().out
    from sharingan import cli
    msg = "the sharingan-1 track closed at 2026-11-16T00:00:00.000Z (the competition's qualifying window); ladder-v1 is open"
    fake_api.refuse = cli.ApiException(403, "competition_closed", msg)
    assert main(["submit", _bot(tmp_path), "--server", "http://x", "--quiet", "--token", "sk_x", "--track", "sharingan-1"]) == 1
    cap = capsys.readouterr()
    assert cap.out == "" and msg in cap.err and "--track ladder-v1" in cap.err


def test_submit_reuses_the_saved_token_of_the_file_name(tmp_path, fake_api, capsys):
    bot = _bot(tmp_path)
    for _ in range(2):
        assert main(["submit", bot, "--server", "http://x", "--quiet"]) == 0
    assert fake_api.registered == ["mybot"], "registered once, then the saved token"
    capsys.readouterr()
    assert main(["token", bot, "--server", "http://x"]) == 0 and capsys.readouterr().out == "sk_1\n"


def test_play_reuses_the_saved_bot_token(tmp_path, fake_api, monkeypatch, capsys):
    from sharingan import cli
    tokens_seen = []

    class FakeRunner:
        def __init__(self, bot, server, token, max_hands=None, **kw):
            tokens_seen.append(token)
            self.hands_played, self.net_chips, self._my_seats = 0, 0, {}

        async def run(self):
            return None

    class FakeTable:
        table_id = "t1"

    monkeypatch.setattr(fake_api, "create_table", lambda self, **kw: FakeTable(), raising=False)
    monkeypatch.setattr(fake_api, "join_table", lambda self, tid, seat, pid, token=None: tokens_seen.append(("seat", tid, seat, pid)),
                        raising=False)
    monkeypatch.setattr(cli, "Runner", FakeRunner)
    for _ in range(3):
        assert main(["play", _bot(tmp_path), "--vs", "sharingan-1", "--server", "http://x"]) == 0
    assert fake_api.registered == ["mybot"], "one registration for three plays"
    assert [t for t in tokens_seen if isinstance(t, str)] == ["sk_1"] * 3
    assert ("seat", "t1", 0, "p_1") in tokens_seen, "the saved player id sits at the table"


def test_load_bot_names_llm_bots_after_their_file_unless_named(tmp_path):
    from sharingan.bots import LLMBot
    head = "from sharingan import Bot\nfrom sharingan.bots import LLMBot\ndef complete(p):\n    return 'ACTION: call'\n"
    for stem, line, want in [
        ("alpha", "bot = LLMBot(complete)", "alpha"),
        ("beta", "bot = LLMBot(complete)", "beta"),
        ("gamma", "bot = LLMBot(complete, name='my llm')", "my llm"),
        ("delta", "bot = LLMBot(complete)\nbot.name = 'chosen'", "chosen"),
        ("eps", "class Mine(Bot):\n    name = 'explicit'\n    def act(self, s): return None\nbot = Mine()", "explicit"),
        ("zeta", "class Mine(Bot):\n    def act(self, s): return None\nbot = Mine()", "zeta"),
    ]:
        f = tmp_path / f"{stem}.py"
        f.write_text(head + line + "\n")
        b = load_bot(str(f))
        assert b.name == want, (stem, b.name)
    assert LLMBot(lambda p: "").name == "bot" and LLMBot(lambda p: "", name="x").name == "x"


def test_webhook_server_accepts_both_signature_headers():
    import json
    import urllib.error
    import urllib.request
    from sharingan.bots import CallingStation
    from sharingan.webhook import serve_webhook, sign
    from conftest import DOC_YOUR_TURN
    body = json.dumps(DOC_YOUR_TURN).encode()
    srv = serve_webhook(CallingStation(), host="127.0.0.1", port=0, secret="s3", block=False)
    try:
        def post(headers):
            req = urllib.request.Request(f"http://127.0.0.1:{srv.server_address[1]}/act", data=body, headers=headers, method="POST")
            try:
                with urllib.request.urlopen(req, timeout=5) as r:
                    return r.status
            except urllib.error.HTTPError as e:
                return e.code
        assert post({"X-Sharingan-Signature": sign("s3", body)}) == 200, "what the server sends"
        assert post({"X-Signature": sign("s3", body)}) == 200, "the old name still works"
        assert post({"X-Sharingan-Signature": "sha256=bad"}) == 401
        assert post({}) == 401
    finally:
        srv.shutdown()
        srv.server_close()
