"""`sharingan` command line: eval / benchmark / connect / play / serve a bot file.

    sharingan eval mybot.py [--hands 3000] [--server http://localhost:8080]   # ladder-v1 benchmark if the server has it
    sharingan benchmark mybot.py --track ladder-v1|sharingan-1 [--hands N] [--server URL] [--token T]
    sharingan connect mybot.py --server URL --token T [--player-id ID]        # answer your_turn until the server closes
    sharingan play mybot.py --vs sharingan-1 [--server URL]
    sharingan serve mybot.py --port 9000 [--secret S]
    sharingan pipe mybot.py                                                   # stdin/stdout, for `sharingan-arena run --bot "sharingan pipe mybot.py"`
    sharingan eval mybot.py --local [--track T] [--hands N] [--lanes L]       # run the local worker binary, no server

`mybot.py` may define a `Bot` subclass (first one defined in the file is used), a module-level `bot`
instance, or a plain `act(state)` function.
"""
from __future__ import annotations

import argparse
import asyncio
import importlib.util
import inspect
import os
import signal
import sys
import time
from typing import Any, List, Optional

from .bot import Bot
from .client import ApiException, ArenaClient, Runner
from .models import Action

DEFAULT_SERVER = "http://localhost:8080"
TRACKS = ("ladder-v1", "sharingan-1")
DEFAULT_TRACK = "ladder-v1"
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


# ----------------------------------------------------------------------------- benchmark / connect

def format_bench(name: str, run: dict, track: str) -> str:
    """One line: mbb/game +- 95% CI (AIVAT) over N hands on the track."""
    mbb, ci = run.get("mbb_per_game"), run.get("ci95_mbb")
    hands = int(run.get("hands_done") or run.get("hands_total") or run.get("hands_planned") or 0)
    if mbb is None:
        mbb, ci = run.get("running_mbb_per_game"), run.get("running_ci95_mbb")
    method = run.get("method")
    tag = " [verified]" if run.get("verified") else " [unverified]"
    return (f"{name}: {float(mbb or 0.0):+.1f} ± {float(ci or 0.0):.1f} mbb/game (95% CI"
            + (f", {method}" if method else "") + f") over {hands} hands on {track}{tag}")


def leaderboard_url(server: str, track: str) -> str:
    return f"{ArenaClient(server).base_url}/v1/leaderboard?track={track}"


class _Progress:
    """One-line live bar on stderr (tty: redrawn in place; otherwise a line per 10%)."""

    def __init__(self, track: str, enabled: bool):
        self.track, self.enabled, self.tty = track, enabled, sys.stderr.isatty()
        self._last_decile = -1
        self._shown = False

    def update(self, done: int, total: int, mbb: Any = None, ci: Any = None) -> None:
        if not self.enabled or not total:
            return
        frac = min(1.0, max(0.0, done / total))
        est = f"  {float(mbb):+.1f} ± {float(ci or 0):.1f} mbb/game" if mbb is not None else ""
        if self.tty:
            n = int(frac * 24)
            print(f"\r  {self.track} [{'#' * n}{'-' * (24 - n)}] {frac * 100:3.0f}% {done}/{total} hands{est}\x1b[K",
                  end="", file=sys.stderr, flush=True)
            self._shown = True
        elif int(frac * 10) > self._last_decile:
            self._last_decile = int(frac * 10)
            print(f"  {self.track}: {frac * 100:.0f}% {done}/{total} hands{est}", file=sys.stderr, flush=True)

    def clear(self) -> None:
        if self.tty and self._shown:
            print("\r\x1b[K", end="", file=sys.stderr, flush=True)
            self._shown = False


def supports_track(api: ArenaClient, track: str) -> bool:
    """True when GET /v1/benchmarks lists `track` as available."""
    try:
        tracks = api.request("GET", "/v1/benchmarks")
    except (ApiException, OSError, ValueError):
        return False
    if isinstance(tracks, dict):
        tracks = tracks.get("tracks") or []
    return any(isinstance(t, dict) and t.get("track") == track and t.get("available", True) for t in tracks or [])


async def run_benchmark(bot: Bot, server: str, track: str, *, hands: Optional[int] = None, token: Optional[str] = None,
                        timeout: float = 0, quiet: bool = False, lanes: Optional[int] = None,
                        connect_timeout: float = 15.0) -> "tuple[int, Optional[dict]]":
    """Register (or reuse `token`), connect to /v1/play, POST /v1/benchmarks/{track}/runs, follow benchmark_progress
    until the run ends. Returns (exit_code, final_run). Errors are printed to stderr."""
    api = ArenaClient(server, token)
    if not token:
        try:
            token = (await asyncio.to_thread(api.register_bot, bot.name)).token or ""
        except OSError as e:
            return _unreachable(server, e), None
        except ApiException as e:
            print(f"sharingan: could not register: {e}", file=sys.stderr)
            return 1, None
    bar = _Progress(track, not quiet)
    done = asyncio.Event()
    state: dict = {"run_id": None}

    def on_msg(env: dict) -> None:
        if env.get("type") != "benchmark_progress":
            return
        d = env.get("data") or {}
        if state["run_id"] and d.get("run_id") not in (None, state["run_id"]):
            return
        bar.update(int(d.get("hands_done") or 0), int(d.get("hands_total") or 0), d.get("mbb_per_game"), d.get("ci95_mbb"))
        if d.get("status") in ("finished", "aborted", "failed"):
            done.set()

    runner = Runner(bot, server, token or "", on_message=on_msg)
    rtask = asyncio.ensure_future(runner.run())
    try:
        t0 = time.time()
        while runner.welcome is None:  # a run can only start once the bot's socket is up
            if rtask.done() or time.time() - t0 > connect_timeout:
                print(f"sharingan: could not connect to {server} /v1/play (bad token?)", file=sys.stderr)
                return 1, None
            await asyncio.sleep(0.05)
        body: dict = {}
        if hands:
            body["hands"] = hands
        if lanes:
            body["lanes"] = lanes
        run = None
        while run is None:
            try:
                run = await asyncio.to_thread(lambda: api.request("POST", f"/v1/benchmarks/{track}/runs", body, token=token))
            except ApiException as e:
                if e.code == "not_connected" and time.time() - t0 < connect_timeout:
                    await asyncio.sleep(0.2)
                    continue
                print(f"sharingan: server refused the run: {e}", file=sys.stderr)
                return 1, None
        state["run_id"] = run["run_id"]
        t1 = time.time()
        while True:
            try:
                await asyncio.wait_for(done.wait(), 1.0)
            except asyncio.TimeoutError:
                pass
            cur = await asyncio.to_thread(lambda: api.request("GET", f"/v1/benchmarks/{track}/runs/{run['run_id']}", token=token))
            if cur.get("status") != "running":
                break
            bar.update(int(cur.get("hands_done") or 0), int(cur.get("hands_total") or 0),
                       cur.get("running_mbb_per_game"), cur.get("running_ci95_mbb"))
            if timeout and time.time() - t1 > timeout:
                bar.clear()
                print(f"sharingan: timed out after {timeout}s ({cur.get('hands_done', 0)}/{cur.get('hands_total', 0)} hands); "
                      f"run {run['run_id']} is still going on the server", file=sys.stderr)
                return 1, cur
        bar.clear()
        if cur.get("status") != "finished":
            print(f"sharingan: run {cur.get('run_id')} ended as {cur.get('status')}", file=sys.stderr)
            return 1, cur
        return 0, cur
    except OSError as e:
        bar.clear()
        return _unreachable(server, e), None
    finally:
        runner.stop()
        await asyncio.gather(rtask, return_exceptions=True)


async def _benchmark(args: argparse.Namespace) -> int:
    bot = load_bot(args.bot)
    rc, run = await run_benchmark(bot, args.server, args.track, hands=args.hands, token=args.token, timeout=args.timeout,
                                  quiet=args.quiet, lanes=args.lanes)
    if rc == 0 and run:
        print(format_bench(bot.name, run, args.track))
        print(f"leaderboard: {leaderboard_url(args.server, args.track)}")
    return rc


async def _connect(args: argparse.Namespace) -> int:
    """Connect an already-registered bot to /v1/play and keep answering your_turn until the server closes
    (or SIGINT/SIGTERM). Starting a benchmark run for it is somebody else's job (`sharingan benchmark`, or the runner)."""
    bot = load_bot(args.bot)

    def on_msg(env: dict) -> None:
        if env.get("type") == "welcome":
            pid = (env.get("data") or {}).get("player_id")
            if args.player_id and pid and pid != args.player_id:
                print(f"sharingan: warning: token belongs to {pid}, not {args.player_id}", file=sys.stderr)
            print(f"sharingan: connected as {pid} ({bot.name})", file=sys.stderr, flush=True)

    runner = Runner(bot, args.server, args.token, max_hands=args.max_hands or None, reconnect=not args.no_reconnect,
                    on_message=on_msg)
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGTERM, signal.SIGINT):
        try:
            loop.add_signal_handler(sig, runner.stop)
        except (NotImplementedError, RuntimeError):  # Windows / non-main thread
            pass
    print(f"sharingan: connecting {bot.name} to {args.server}", file=sys.stderr, flush=True)
    try:
        await runner.run()
    except OSError as e:
        return _unreachable(args.server, e)
    if runner.welcome is None:
        print(f"sharingan: never connected to {args.server}", file=sys.stderr)
        return 1
    print(f"sharingan: disconnected after {runner.hands_played} hands", file=sys.stderr)
    return 0


async def _eval(args: argparse.Namespace) -> int:
    if getattr(args, "local", False):
        return await asyncio.to_thread(_eval_local, args)
    bot = load_bot(args.bot)
    api = ArenaClient(args.server)
    # Default: the ladder-v1 benchmark track (AIVAT-corrected, shows up on the leaderboard) when the server has it.
    if not args.match and args.vs in (None, "ladder"):
        ok = await asyncio.to_thread(supports_track, api, DEFAULT_TRACK)
        if ok:
            rc, run = await run_benchmark(bot, args.server, DEFAULT_TRACK, hands=max(6, (args.hands // 6) * 6),
                                          timeout=args.timeout, quiet=args.quiet)
            if rc == 0 and run:
                print(format_bench(bot.name, run, DEFAULT_TRACK))
            return rc
    args.vs = args.vs or "ladder"
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
    args.vs = args.vs or "ladder"
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


def _pipe(args: argparse.Namespace) -> int:
    """`sharingan pipe mybot.py`: serve the bot over stdin/stdout for `sharingan-arena run --bot "sharingan pipe mybot.py"`."""
    from .pipe import claim_stdio, run_pipe
    r, w = claim_stdio()  # before importing the bot: its import-time prints must not reach the protocol stream
    return run_pipe(load_bot(args.bot), r, w)


def find_worker() -> Optional[str]:
    """The match-worker binary: $SHARINGAN_ARENA_BIN, else `sharingan-arena` on PATH."""
    import shutil
    exe = os.environ.get("SHARINGAN_ARENA_BIN")
    if exe and os.access(exe, os.X_OK):
        return exe
    return shutil.which("sharingan-arena")


NO_WORKER_MSG = ("sharingan: --local needs the match worker binary `sharingan-arena` on PATH (or SHARINGAN_ARENA_BIN=/path).\n"
                 "Build it with:  cargo build --release -p sharingan-arena   (then put target/release on PATH),\n"
                 "or evaluate against a server instead:  sharingan eval mybot.py --server URL")


def _eval_local(args: argparse.Namespace) -> int:
    """Run the worker binary locally: the bot plays as a pipe subprocess, no server needed."""
    import json
    import shlex
    import subprocess
    import tempfile
    exe = find_worker()
    if not exe:
        print(NO_WORKER_MSG, file=sys.stderr)
        return 2
    path = os.path.abspath(args.bot)
    if not os.path.isfile(path):
        print(f"sharingan: no such file: {path}", file=sys.stderr)
        return 2
    track = args.track or DEFAULT_TRACK
    env = dict(os.environ)
    pkg_parent = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    env["PYTHONPATH"] = pkg_parent + (os.pathsep + env["PYTHONPATH"] if env.get("PYTHONPATH") else "")
    bot_cmd = f"{shlex.quote(sys.executable)} -m sharingan.cli pipe {shlex.quote(path)}"
    out = args.out or os.path.join(tempfile.mkdtemp(prefix="sharingan-"), "result.json")
    cmd = [exe, "run", "--bot", bot_cmd, "--track", track, "--hands", str(max(6, args.hands)), "--out", out,
           "--decision-timeout-ms", str(args.decision_ms)]
    if args.lanes:
        cmd += ["--lanes", str(args.lanes)]
    if args.seed_set:
        cmd += ["--seed", args.seed]
    if args.quiet:
        cmd += ["--quiet"]
    rc = subprocess.call(cmd, env=env)
    if rc != 0:
        return rc
    with open(out) as f:
        res = json.load(f)
    print(f"{os.path.splitext(os.path.basename(path))[0]}: {float(res['mbb_per_game']):+.1f} ± {float(res['ci95_mbb']):.1f} mbb/game "
          f"(95% CI, {res.get('method')}) over {res['hands']} hands on {res['track']} [local, unverified; "
          f"{res['timings']['hands_per_sec']:.0f} hands/s, result: {out}]"
          + (f" WARNING {res['forfeit_hands']} forfeit hands (bot crashed or timed out)" if res.get("forfeit_hands") else ""))
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
        sp.add_argument("--vs", default=None, help="ladder (default) | sharingan-1 | random | calling_station | tight_aggressive | <house bot>")
        sp.add_argument("--decision-ms", type=int, default=5000)

    e = sub.add_parser("eval", help="ladder-v1 benchmark (or a duplicate match vs house bots on older servers); prints one score line")
    common(e)
    e.add_argument("--match", action="store_true", help="force the plain duplicate match instead of the ladder-v1 benchmark")
    e.add_argument("--hands", type=int, default=3000, help="hands to play (rounded down to a multiple of 6)")
    e.add_argument("--seed", default="sharingan-eval", help="match seed (same seed = same decks)")
    e.add_argument("--timeout", type=float, default=0, help="give up after N seconds (0 = wait)")
    e.add_argument("--quiet", action="store_true")
    e.add_argument("--local", action="store_true",
                   help="no server: run the `sharingan-arena` worker binary from PATH and play the bot as a pipe subprocess")
    e.add_argument("--track", choices=TRACKS, default=None, help="with --local: ladder-v1 (default) | sharingan-1")
    e.add_argument("--lanes", type=int, default=0, help="with --local: parallel bot processes (default: worker decides)")
    e.add_argument("--out", default=None, help="with --local: where to write result.json")
    pp = sub.add_parser("pipe", help="serve the bot over stdin/stdout (pipe protocol) for `sharingan-arena run --bot \"sharingan pipe mybot.py\"`")
    pp.add_argument("bot")
    b = sub.add_parser("benchmark", help="run a benchmark track (ladder-v1 | sharingan-1) and print mbb/game ± CI")
    b.add_argument("bot")
    b.add_argument("--track", choices=TRACKS, default=DEFAULT_TRACK)
    b.add_argument("--hands", type=int, default=None, help="hands (server default 20000; rounded up to a multiple of 6)")
    b.add_argument("--lanes", type=int, default=None, help="parallel lanes (bot must key state by table_id/hand_id)")
    b.add_argument("--server", default=os.environ.get("SHARINGAN_SERVER", DEFAULT_SERVER))
    b.add_argument("--token", default=os.environ.get("SHARINGAN_TOKEN"), help="reuse a registered bot instead of registering a new one")
    b.add_argument("--timeout", type=float, default=0, help="give up waiting after N seconds (0 = wait)")
    b.add_argument("--quiet", action="store_true", help="no progress bar")
    c = sub.add_parser("connect", help="connect an already-registered bot and answer your_turn until the server closes")
    c.add_argument("bot")
    c.add_argument("--server", default=os.environ.get("SHARINGAN_SERVER", DEFAULT_SERVER))
    c.add_argument("--token", default=os.environ.get("SHARINGAN_TOKEN"), required=not os.environ.get("SHARINGAN_TOKEN"))
    c.add_argument("--player-id", default=None, help="expected player id (warns if the token belongs to another)")
    c.add_argument("--max-hands", type=int, default=0, help="disconnect after N hands (0 = no limit)")
    c.add_argument("--no-reconnect", action="store_true", help="exit on the first disconnect instead of reconnecting")
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
    if a.cmd == "eval":
        a.seed_set = any(x == "--seed" or x.startswith("--seed=") for x in (argv if argv is not None else sys.argv[1:]))
    try:
        if a.cmd == "pipe":
            return _pipe(a)
        if a.cmd == "eval":
            return asyncio.run(_eval(a))
        if a.cmd == "benchmark":
            return asyncio.run(_benchmark(a))
        if a.cmd == "connect":
            return asyncio.run(_connect(a))
        if a.cmd == "play":
            return asyncio.run(_play(a))
        return _serve(a)
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
