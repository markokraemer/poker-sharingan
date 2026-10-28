"""`sharingan` command line: eval / play / serve a bot file.

    sharingan eval mybot.py [--hands 3000] [--vs ladder] [--server http://localhost:8080]
    sharingan play mybot.py --vs sharingan-1 [--server URL]
    sharingan serve mybot.py --port 9000 [--secret S]

`mybot.py` may define a `Bot` subclass (first one defined in the file is used), a module-level `bot`
instance, or a plain `act(state)` function.
"""
from __future__ import annotations

import argparse
import asyncio
import importlib.util
import inspect
import os
import sys
import time
from typing import Any, List, Optional

from .bot import Bot
from .client import ApiException, ArenaClient, Runner
from .models import Action

DEFAULT_SERVER = "http://localhost:8080"
START_CMD = "cargo run --release -p sharingan-arena -- --port 8080"
# House bots are addressed as "house:<name>" in `participants` / seat player_ids (assumption, see SDK notes).
LADDER = ["random", "calling_station", "tight_aggressive", "tight_aggressive", "sharingan-1"]


class FunctionBot(Bot):
    def __init__(self, fn: Any, name: str):
        self._fn, self.name = fn, name
        if inspect.iscoroutinefunction(fn):
            self.act = fn  # type: ignore[assignment]

    def act(self, state):  # type: ignore[override]
        return self._fn(state)


def load_bot(path: str) -> Bot:
    """Import `path` and return a Bot: instance named `bot`, else first Bot subclass defined there, else `act()`."""
    if not os.path.isfile(path):
        raise SystemExit(f"sharingan: no such file: {path}")
    name = os.path.splitext(os.path.basename(path))[0]
    spec = importlib.util.spec_from_file_location(f"_sharingan_user_{name}", path)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.path.insert(0, os.path.dirname(os.path.abspath(path)))
    spec.loader.exec_module(mod)
    inst = getattr(mod, "bot", None)
    if isinstance(inst, Bot):
        return inst
    classes = [c for c in vars(mod).values()
               if inspect.isclass(c) and issubclass(c, Bot) and c is not Bot and c.__module__ == mod.__name__]
    if classes:
        b = classes[0]()
        if b.name == "bot":
            b.name = name
        return b
    fn = getattr(mod, "act", None)
    if callable(fn):
        return FunctionBot(fn, name)
    raise SystemExit(f"sharingan: {path} defines no Bot subclass, `bot` instance or `act(state)` function")


def _house(vs: str) -> List[str]:
    names = LADDER if vs == "ladder" else [vs] * 5
    return [f"house:{n}" for n in names]


def _unreachable(server: str, err: Exception) -> int:
    print(f"sharingan: arena server not reachable at {server} ({err}).\n"
          f"Start one with:  {START_CMD}\n(or pass --server URL)", file=sys.stderr)
    return 2


def _score(match: dict, player_id: str) -> Optional[dict]:
    st = match.get("standings") or []
    if isinstance(st, dict):
        st = st.get("players") or list(st.values())
    for s in st:
        if s.get("player_id") == player_id:
            return s
    return None


def format_score(name: str, s: dict, hands: int, vs: str) -> str:
    """One line: mbb/game +- 95% CI. 1 bb/100 == 10 mbb/hand. Uses the duplicate score when the server reports it."""
    bb100 = s.get("duplicate_bb_per_100")
    dup = bb100 is not None
    if bb100 is None:
        bb100 = s.get("bb_per_100")
    mbb = bb100 * 10.0 if bb100 is not None else float(s.get("mbb_per_hand", 0.0))
    ci = float(s.get("ci95_bb_per_100", 0.0)) * 10.0
    return f"{name}: {mbb:+.1f} ± {ci:.1f} mbb/game (95% CI) over {hands} hands vs {vs}" + (" [duplicate]" if dup else "")


async def _eval(args: argparse.Namespace) -> int:
    bot = load_bot(args.bot)
    api = ArenaClient(args.server)
    try:
        me = await asyncio.to_thread(api.register_bot, bot.name)
    except OSError as e:  # URLError, ConnectionRefusedError
        return _unreachable(args.server, e)
    hands = max(6, (args.hands // 6) * 6)  # whole deals: 6 rotations each
    runner = Runner(bot, args.server, me.token or "")
    rtask = asyncio.ensure_future(runner.run())
    try:
        cfg = {"variant": "nlhe", "max_seats": 6, "small_blind": 50, "big_blind": 100, "ante": 0, "straddle": False,
               "starting_stack": 10000, "stack_mode": "reset", "decision_time_ms": args.decision_ms,
               "time_bank_ms": 0, "timeout_action": "check_fold", "rake": {"percent": 0, "cap": 0},
               "min_players_to_deal": 6, "odd_chip_rule": "first_left_of_button", "show_mode": "standard"}
        body = dict(table_config=cfg, participants=[me.player_id] + _house(args.vs), hands_planned=hands,
                    duplicate={"enabled": True, "rotations": 6, "deck_from": "seed"}, seed=args.seed)
        try:
            m = await asyncio.to_thread(lambda: api.create_match(**body))
            await asyncio.to_thread(api.match_action, m["match_id"], "start")
        except ApiException as e:
            print(f"sharingan: server refused the match: {e}", file=sys.stderr)
            return 1
        t0 = time.time()
        while True:
            cur = await asyncio.to_thread(api.get_match, m["match_id"])
            if cur.get("status") in ("finished", "aborted"):
                break
            if args.timeout and time.time() - t0 > args.timeout:
                print(f"sharingan: timed out after {args.timeout}s ({cur.get('hands_played', 0)}/{hands} hands)", file=sys.stderr)
                return 1
            if not args.quiet and sys.stderr.isatty():
                print(f"\r  {cur.get('hands_played', 0)}/{hands} hands", end="", file=sys.stderr)
            await asyncio.sleep(0.5)
        if sys.stderr.isatty():
            print("\r" + " " * 30 + "\r", end="", file=sys.stderr)
        s = _score(cur, me.player_id)
        if not s:
            print("sharingan: match finished but no standings for this bot", file=sys.stderr)
            return 1
        print(format_score(bot.name, s, int(cur.get("hands_played", hands)), args.vs))
        return 0
    finally:
        runner.stop()
        await asyncio.gather(rtask, return_exceptions=True)


async def _play(args: argparse.Namespace) -> int:
    bot = load_bot(args.bot)
    api = ArenaClient(args.server)
    try:
        me = await asyncio.to_thread(api.register_bot, bot.name)
        if args.table:
            tid = args.table
            await asyncio.to_thread(api.join_table, tid, args.seat, me.player_id)
        else:
            t = await asyncio.to_thread(api.create_table, decision_time_ms=args.decision_ms)
            tid = t.table_id
            await asyncio.to_thread(api.join_table, tid, 0, me.player_id)
            for i, h in enumerate(_house(args.vs), start=1):  # house bots fill the empty seats
                await asyncio.to_thread(api.join_table, tid, i, h)
    except OSError as e:
        return _unreachable(args.server, e)
    except ApiException as e:
        print(f"sharingan: {e}", file=sys.stderr)
        return 1
    print(f"table {tid}: {bot.name} vs {args.vs}. Ctrl-C to stop.", file=sys.stderr)

    runner = Runner(bot, args.server, me.token or "", max_hands=args.hands or None)
    orig = getattr(bot, "on_hand_end", None)

    def on_end(e):  # print one line per hand, then defer to the bot's own hook
        seat = runner._my_seats.get(tid)
        print(f"hand {runner.hands_played + 1}: {e.net_for(seat) if seat is not None else '?':+} chips  (total {runner.net_chips:+})"
              if seat is not None else f"hand {runner.hands_played + 1} ended")
        return orig(e) if orig else None
    bot.on_hand_end = on_end  # type: ignore[method-assign]
    try:
        await runner.run()
    except KeyboardInterrupt:
        pass
    print(f"{runner.hands_played} hands, net {runner.net_chips:+} chips")
    return 0


def _serve(args: argparse.Namespace) -> int:
    from .webhook import serve_webhook
    bot = load_bot(args.bot)
    print(f"serving {bot.name} as a webhook on http://0.0.0.0:{args.port}/act"
          + (" (HMAC signature required)" if args.secret else "") + "\nRegister it with:  "
          f'POST /v1/players {{"name": "{bot.name}", "kind": "bot", "webhook_url": "https://YOUR_HOST:{args.port}/act", "secret": "..."}}',
          file=sys.stderr)
    serve_webhook(bot, host=args.host, port=args.port, secret=args.secret)
    return 0


def main(argv: Optional[List[str]] = None) -> int:
    p = argparse.ArgumentParser(prog="sharingan", description="Eval and play poker bots on the Sharingan arena.")
    sub = p.add_subparsers(dest="cmd", required=True)

    def common(sp: argparse.ArgumentParser) -> None:
        sp.add_argument("bot", help="path to mybot.py (Bot subclass or act(state) function)")
        sp.add_argument("--server", default=os.environ.get("SHARINGAN_SERVER", DEFAULT_SERVER))
        sp.add_argument("--vs", default="ladder", help="ladder | sharingan-1 | random | calling_station | tight_aggressive | <house bot>")
        sp.add_argument("--decision-ms", type=int, default=5000)

    e = sub.add_parser("eval", help="duplicate match vs house bots; prints one score line")
    common(e)
    e.add_argument("--hands", type=int, default=3000, help="hands to play (rounded down to a multiple of 6)")
    e.add_argument("--seed", default="sharingan-eval", help="match seed (same seed = same decks)")
    e.add_argument("--timeout", type=float, default=0, help="give up after N seconds (0 = wait)")
    e.add_argument("--quiet", action="store_true")
    pl = sub.add_parser("play", help="join a table; house bots fill the empty seats")
    common(pl)
    pl.add_argument("--table", help="join this existing table instead of creating one")
    pl.add_argument("--seat", type=int, default=0)
    pl.add_argument("--hands", type=int, default=0, help="stop after N hands (0 = until Ctrl-C)")
    s = sub.add_parser("serve", help="expose the bot as a webhook endpoint")
    s.add_argument("bot")
    s.add_argument("--port", type=int, default=9000)
    s.add_argument("--host", default="0.0.0.0")
    s.add_argument("--secret", default=os.environ.get("SHARINGAN_WEBHOOK_SECRET"))
    a = p.parse_args(argv)
    try:
        if a.cmd == "eval":
            return asyncio.run(_eval(a))
        if a.cmd == "play":
            return asyncio.run(_play(a))
        return _serve(a)
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
