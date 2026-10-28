"""Pipe runner: run a `Bot` as a subprocess of the match worker, talking newline-delimited JSON over stdin/stdout.

    sharingan-arena run --bot "sharingan pipe mybot.py" --track ladder-v1 --hands 20000 --lanes 8 --out result.json

The messages are the WebSocket protocol's own (`your_turn`, `hand_end`, `act`, ...), one JSON object per line
(docs/TABLE_API.md section 14). The same `mybot.py` runs unchanged over WebSocket, webhook and pipe.

stdout belongs to the protocol: `claim_stdio()` keeps the real stdout for it and points fd 1 (and `sys.stdout`) at
stderr, so a stray `print()` in your bot cannot corrupt the stream.
"""
from __future__ import annotations

import asyncio
import inspect
import json
import os
import sys
import traceback
from typing import Any, BinaryIO, Optional, Tuple

from .bot import Bot
from .helpers import legalize, safe_action
from .models import Action, ApiError, HandEnd, YourTurn

PIPE_PROTOCOL = 1
CLIENT_NAME = "sharingan-py-pipe/0.1.0"

_dumps = json.JSONEncoder(separators=(",", ":")).encode
_loads = json.loads


def claim_stdio() -> Tuple[BinaryIO, BinaryIO]:
    """Return (reader, writer) for the protocol and redirect everything else that writes to stdout to stderr."""
    out = os.fdopen(os.dup(1), "wb", buffering=0)
    os.dup2(2, 1)
    sys.stdout = sys.stderr
    return sys.stdin.buffer, out


def _overrides(bot: Bot, hook: str) -> bool:
    return getattr(type(bot), hook, None) is not getattr(Bot, hook, None) or hook in vars(bot)


class PipeRunner:
    def __init__(self, bot: Bot, rfile: BinaryIO, wfile: BinaryIO):
        self.bot, self.r, self.w = bot, rfile, wfile
        self._loop: Optional[asyncio.AbstractEventLoop] = None
        self._want_end = _overrides(bot, "on_hand_end")
        self._want_error = _overrides(bot, "on_error")
        self.decisions = 0

    def send(self, type_: str, data: dict, table_id: Optional[str] = None) -> None:
        env: dict = {"v": 1, "type": type_}
        if table_id:
            env["table_id"] = table_id
        env["data"] = data
        self.w.write((_dumps(env) + "\n").encode())

    def _call(self, fn: Any, *args: Any) -> Any:
        res = fn(*args)
        if inspect.isawaitable(res):
            if self._loop is None:
                self._loop = asyncio.new_event_loop()
            res = self._loop.run_until_complete(res)
        return res

    def _hook(self, fn: Any, *args: Any) -> None:
        try:
            self._call(fn, *args)
        except Exception:
            traceback.print_exc(file=sys.stderr)

    def run(self) -> int:
        bot = self.bot
        self.send("hello", {"client": CLIENT_NAME, "protocol": PIPE_PROTOCOL, "bot": getattr(bot, "name", "bot")})
        for line in self.r:
            if not line.strip():
                continue
            try:
                env = _loads(line)
            except ValueError:
                continue
            t = env.get("type")
            if t == "your_turn":
                self._turn(env)
            elif t == "hand_end":
                if self._want_end:
                    self._hook(bot.on_hand_end, HandEnd.from_dict(env.get("data") or {}))
            elif t == "welcome":
                self._hook(bot.on_connect, env.get("data") or {})
            elif t == "error":
                if self._want_error:
                    self._hook(bot.on_error, ApiError.from_dict(env.get("data") or {}))
            elif t == "bye":
                self.send("bye", {})
                return 0
        return 0

    def _turn(self, env: dict) -> None:
        d = env.get("data") or {}
        state = YourTurn.from_dict(d, table_id=env.get("table_id", ""), t_ms=int(env.get("t_ms") or 0),
                                   hand_id=env.get("hand_id", ""), seq=int(env.get("seq") or 0))
        self.decisions += 1
        action: Any = None
        try:
            action = self._call(self.bot.act, state)
        except Exception:
            print("sharingan pipe: bot.act raised; playing check/fold", file=sys.stderr)
            traceback.print_exc(file=sys.stderr)
        if not isinstance(action, Action):
            action = safe_action(state)
        elif not getattr(self.bot, "strict", False):
            action = legalize(state, action)
        self.send("act", action.to_data(state.hand_id, state.decision_id), state.table_id or None)


def run_pipe(bot: Bot, rfile: Optional[BinaryIO] = None, wfile: Optional[BinaryIO] = None) -> int:
    """Serve `bot` over the pipe protocol until `bye` or EOF. Returns the exit code."""
    if rfile is None or wfile is None:
        rfile, wfile = claim_stdio()
    try:
        return PipeRunner(bot, rfile, wfile).run()
    except (BrokenPipeError, KeyboardInterrupt):
        return 0
