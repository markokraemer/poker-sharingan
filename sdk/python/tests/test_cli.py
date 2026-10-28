import asyncio
import subprocess
import sys

import pytest

from sharingan.cli import FunctionBot, load_bot, main, format_score
from sharingan.mock import MockArena
from sharingan import Bot


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


def test_eval_prints_one_line(tmp_path, capsys):
    bot = tmp_path / "mybot.py"
    bot.write_text("def act(s):\n    return None\n")

    # main() uses asyncio.run itself, so drive the mock from a background loop thread instead
    import threading
    ready, stop, box = threading.Event(), threading.Event(), {}

    def serve():
        async def run():
            async with MockArena() as arena:
                box["url"] = arena.http_url
                ready.set()
                while not stop.is_set():
                    await asyncio.sleep(0.05)
        asyncio.run(run())
    t = threading.Thread(target=serve, daemon=True)
    t.start()
    assert ready.wait(10)
    try:
        rc = main(["eval", str(bot), "--server", box["url"], "--hands", "600"])
    finally:
        stop.set()
        t.join(5)
    out = capsys.readouterr().out.strip().splitlines()
    assert rc == 0 and out == ["mybot: +40.0 ± 25.0 mbb/game (95% CI) over 600 hands vs ladder [duplicate]"]


def test_unreachable_prints_start_command(tmp_path, capsys):
    bot = tmp_path / "mybot.py"
    bot.write_text("def act(s):\n    return None\n")
    rc = main(["eval", str(bot), "--server", "http://127.0.0.1:1"])
    assert rc == 2 and "cargo run --release -p sharingan-arena" in capsys.readouterr().err
