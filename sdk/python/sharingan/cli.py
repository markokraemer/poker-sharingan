"""`sharingan` command line: eval / benchmark / connect / play / serve a bot file.

    sharingan eval mybot.py [--hands 3000] [--server https://arena.sharingan.poker]   # ladder-v1 benchmark if the server has it
    sharingan benchmark mybot.py --track ladder-v1|sharingan-1 [--hands N] [--server URL] [--token T]
    sharingan connect mybot.py --server URL --token T [--player-id ID]        # answer your_turn until the server closes
    sharingan play mybot.py --vs sharingan-1 [--server URL]
    sharingan serve mybot.py --port 9000 [--secret S]
    sharingan submit mybot.py --contact EMAIL --profile URL [--token T] [--hands N]   # hosted job on the server's runner: verified score
    sharingan token [mybot.py|NAME]                                           # print the saved bot token (for curl: $(sharingan token mybot.py))
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
import json
import os
import re
import signal
import sys
import time
from typing import Any, List, Optional
from urllib.parse import urlparse

from .bot import Bot
from .client import ApiException, ArenaClient, Runner
from .models import Action

DEFAULT_SERVER = "https://arena.sharingan.poker"   # the public arena; self-hosters pass --server or SHARINGAN_SERVER
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
        if inst.name == Bot.name:    # nobody named it: it is called after its file (one bot name = one token = one leaderboard row)
            inst.name = name
        return inst
    classes = [c for c in vars(mod).values()
               if inspect.isclass(c) and issubclass(c, Bot) and c is not Bot and c.__module__ == mod.__name__]
    if classes:
        b = classes[0]()
        if b.name == Bot.name:
            b.name = name
        return b
    fn = getattr(mod, "act", None)
    if callable(fn):
        return FunctionBot(fn, name)
    raise SystemExit(f"sharingan: {path} defines no Bot subclass, `bot` instance or `act(state)` function")


def _house(vs: str) -> List[str]:
    names = LADDER if vs == "ladder" else [vs] * 5
    return [f"house:{n}" for n in names]


def _warn_bot_trouble(job: dict) -> None:
    """A score can be VERIFIED and still come from a bot that crashed or timed out (those hands play check/fold or are
    forfeited). Say so, with the first error line, instead of a clean-looking score."""
    forfeits = job.get("forfeit_hands") or 0
    tail = job.get("bot_log_tail") or ""
    # the exception line itself ("ValueError: ..."), not the traceback header or the source line that raised it
    errs = [ln.strip() for ln in tail.splitlines()
            if re.match(r"^(lane \d+: )?[A-Za-z_][\w.]*(Error|Exception)\b", ln.strip()) or re.search(r"FORFEIT|timed? ?out", ln)]
    if not forfeits and not errs and "Traceback" not in tail:
        return
    total = job.get("hands_done") or job.get("hands_total") or 0
    msg = f"sharingan: WARNING: your bot had trouble in this run"
    if forfeits:
        msg += f": {forfeits} of {total} hands forfeited (crash or over the time limit)"
    print(msg, file=sys.stderr)
    if errs:
        print(f"sharingan: first error: {errs[0][:300]}", file=sys.stderr)
    jid = job.get("id")
    if jid:
        print(f"sharingan: the bot's full log (stderr, tracebacks) is `bot_log_tail` in: curl -s <server>/v1/jobs/{jid} "
              f"-H \"authorization: Bearer $(sharingan token <bot>)\"", file=sys.stderr)
    print("sharingan: fix it and resubmit; try it locally with `sharingan play <bot> --hands 600` (rare streets need many hands)", file=sys.stderr)


def _unreachable(server: str, err: Exception) -> int:
    hint = f"Start one with:  {START_CMD}\n(or pass --server URL)" if re.match(r"https?://(localhost|127\.)", server) \
        else "Check your connection, or pass --server URL for a self-hosted arena."
    print(f"sharingan: arena server not reachable at {server} ({err}).\n{hint}", file=sys.stderr)
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
    base = ArenaClient(server).base_url
    if base.rstrip("/") == "https://arena.sharingan.poker":
        return "https://sharingan.poker/leaderboard/"
    return f"{base}/v1/leaderboard?track={track}"


TOKENS_FILE = os.path.join(os.environ.get("XDG_CONFIG_HOME") or os.path.expanduser("~/.config"), "sharingan", "tokens.json")


def _load_tokens() -> dict:
    try:
        with open(TOKENS_FILE) as f:
            d = json.load(f)
        return d if isinstance(d, dict) else {}
    except (OSError, ValueError):
        return {}


OWNER_FILE = os.path.join(os.path.dirname(TOKENS_FILE), "owner.json")
RANKED_HANDS = 20004    # the public arena ranks verified runs of at least this many hands (ARENA_RANKED_MIN_HANDS)
MIN_HANDS, MAX_HANDS = 600, 200000   # what a hosted job may ask for (the server rounds up to whole deals of 6)
_EMAIL = re.compile(r"^[^\s@]+@[^\s@]+\.[^\s@]+$")
_HOST = re.compile(r"^([a-z0-9-]+\.)+[a-z]{2,}$")


def _profile_ok(s: str) -> bool:
    """Same test as the server: a public http(s) link (scheme optional, `@handle` means X); no credentials, port or IP."""
    if re.match(r"^@\w{1,15}$", s):
        return True
    try:
        u = urlparse(s if re.match(r"^[a-z][a-z0-9+.-]*://", s, re.I) else "https://" + s)
        host, port = (u.hostname or "").lower(), u.port
    except ValueError:
        return False
    return (u.scheme in ("http", "https") and not (u.username or u.password or port)
            and bool(_HOST.match(host)) and not host.endswith(".local"))


def _owner(args: argparse.Namespace) -> Optional[dict]:
    """Who is behind the bot: --contact (email) and --profile (X / LinkedIn / GitHub / personal page link), else the
    environment, else what the last accepted submit saved. None (after explaining) when either is missing or malformed."""
    try:
        with open(OWNER_FILE) as f:
            saved = json.load(f)
        saved = saved if isinstance(saved, dict) else {}
    except (OSError, ValueError):
        saved = {}
    contact = (args.contact or saved.get("contact") or "").strip()
    profile = (args.profile or saved.get("profile") or "").strip()
    if not contact or not profile:
        print("sharingan: every submit needs who is behind the bot (never shown publicly):\n"
              "  --contact you@example.com           an email to reach you (competition prizes)\n"
              "  --profile https://x.com/you         your X, LinkedIn, GitHub or personal page\n"
              f"Pass them once; they are saved in {OWNER_FILE} (or set SHARINGAN_CONTACT / SHARINGAN_PROFILE).",
              file=sys.stderr)
        return None
    if not _EMAIL.match(contact):
        print(f"sharingan: --contact must be an email address, got {contact!r}", file=sys.stderr)
        return None
    if not _profile_ok(profile):
        print(f"sharingan: --profile must be a link to your X, LinkedIn, GitHub or personal page, like https://x.com/you; got {profile!r}",
              file=sys.stderr)
        return None
    return {"contact": contact, "profile": profile}


def _save_owner(owner: dict) -> None:
    try:
        os.makedirs(os.path.dirname(OWNER_FILE), exist_ok=True)
        fd = os.open(OWNER_FILE + ".tmp", os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, "w") as f:
            json.dump(owner, f, indent=1)
        os.replace(OWNER_FILE + ".tmp", OWNER_FILE)
    except OSError:
        pass


def bot_login(api: ArenaClient, name: str) -> "tuple[str, str]":
    """(player_id, token) of bot `name` on this server: the saved one, or register the bot once and save its token (0600), so
    every eval / benchmark / play / submit of the same bot lands on the same leaderboard row (and burns no new registration).
    The client is left authenticated as that bot."""
    key = f"{api.base_url}|{name}"
    saved = _load_tokens().get(key)
    if isinstance(saved, dict) and saved.get("token"):
        try:
            me = api.request("GET", "/v1/me", token=saved["token"])
            pid = (me.get("player_id") if isinstance(me, dict) else None) or saved.get("player_id") or ""
            api.token = saved["token"]
            return pid, saved["token"]
        except ApiException as e:
            if e.status != 401:
                api.token = saved["token"]
                return saved.get("player_id") or "", saved["token"]
    me = api.register_bot(name)
    tokens = _load_tokens()
    tokens[key] = {"player_id": me.player_id, "token": me.token}
    try:
        os.makedirs(os.path.dirname(TOKENS_FILE), exist_ok=True)
        fd = os.open(TOKENS_FILE + ".tmp", os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, "w") as f:
            json.dump(tokens, f, indent=1)
        os.replace(TOKENS_FILE + ".tmp", TOKENS_FILE)
        where = f"saved to {TOKENS_FILE}"
    except OSError:
        where = "NOT saved, keep it"
    print(f"sharingan: registered {name} as {me.player_id}; token {me.token} ({where})", file=sys.stderr)
    return me.player_id, me.token or ""


def bot_token(api: ArenaClient, name: str) -> str:
    """The saved token for bot `name` on this server, registering the bot once if there is none (see `bot_login`)."""
    return bot_login(api, name)[1]


def _bot_name_for(arg: str) -> str:
    """The bot name a CLI argument stands for: a bot file (or directory) is named after its stem, anything else is a name."""
    if os.path.exists(arg) or arg.endswith(".py") or os.sep in arg:
        return os.path.splitext(os.path.basename(arg.rstrip("/\\")))[0][:40] or "bot"
    return arg


def _token(args: argparse.Namespace) -> int:
    """`sharingan token [BOT]`: print the saved token and nothing else, so `$(sharingan token mybot.py)` works in a shell."""
    base = ArenaClient(args.server).base_url
    prefix = base + "|"
    mine = {k[len(prefix):]: v["token"] for k, v in _load_tokens().items()
            if k.startswith(prefix) and isinstance(v, dict) and v.get("token")}
    if args.bot:
        name = _bot_name_for(args.bot)
        if name in mine:
            print(mine[name])
            return 0
        have = f" Saved for this server: {', '.join(sorted(mine))}." if mine else ""
        print(f"sharingan: no saved token for bot {name!r} on {base}.{have}\n"
              "It is saved the first time you run `sharingan submit`, `benchmark`, `eval` or `play` with that bot.", file=sys.stderr)
        return 1
    if len(mine) == 1:
        print(next(iter(mine.values())))
        return 0
    if not mine:
        print(f"sharingan: no saved token for {base} (looked in {TOKENS_FILE}). "
              "Run `sharingan submit mybot.py` first, or pass --server.", file=sys.stderr)
    else:
        print(f"sharingan: {len(mine)} bots have a saved token on {base}: {', '.join(sorted(mine))}. "
              "Say which one: `sharingan token mybot.py`.", file=sys.stderr)
    return 1


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
            token = await asyncio.to_thread(bot_token, api, bot.name)
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
        pid, token = await asyncio.to_thread(bot_login, api, bot.name)
    except OSError as e:  # URLError, ConnectionRefusedError
        return _unreachable(args.server, e)
    except ApiException as e:
        print(f"sharingan: could not register: {e}", file=sys.stderr)
        return 1
    hands = max(6, (args.hands // 6) * 6)  # whole deals: 6 rotations each
    runner = Runner(bot, args.server, token)
    rtask = asyncio.ensure_future(runner.run())
    try:
        cfg = {"variant": "nlhe", "max_seats": 6, "small_blind": 50, "big_blind": 100, "ante": 0, "straddle": False,
               "starting_stack": 10000, "stack_mode": "reset", "decision_time_ms": args.decision_ms,
               "time_bank_ms": 0, "timeout_action": "check_fold", "rake": {"percent": 0, "cap": 0},
               "min_players_to_deal": 6, "odd_chip_rule": "first_left_of_button", "show_mode": "standard"}
        body = dict(table_config=cfg, participants=[pid] + _house(args.vs), hands_planned=hands,
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
        s = _score(cur, pid)
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
        pid, token = await asyncio.to_thread(bot_login, api, bot.name)   # the saved token: no new registration per run
        if args.table:
            tid = args.table
            await asyncio.to_thread(api.join_table, tid, args.seat, pid)
        else:
            t = await asyncio.to_thread(api.create_table, decision_time_ms=args.decision_ms)
            tid = t.table_id
            await asyncio.to_thread(api.join_table, tid, 0, pid)
            for i, h in enumerate(_house(args.vs), start=1):  # house bots fill the empty seats
                await asyncio.to_thread(api.join_table, tid, i, h)
    except OSError as e:
        return _unreachable(args.server, e)
    except ApiException as e:
        print(f"sharingan: {e}", file=sys.stderr)
        return 1
    print(f"table {tid}: {bot.name} vs {args.vs}. Ctrl-C to stop.", file=sys.stderr)

    runner = Runner(bot, args.server, token, max_hands=args.hands or None)
    orig = getattr(bot, "on_hand_end", None)

    def on_end(e):  # print one line per hand, then defer to the bot's own hook
        seat = runner._my_seats.get(tid)
        print(f"hand {runner.hands_played}: {e.net_for(seat) if seat is not None else '?':+} chips  (total {runner.net_chips:+})"
              if seat is not None else f"hand {runner.hands_played} ended")
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


def _submit(args: argparse.Namespace) -> int:
    """Hosted job: upload the bot, the server's runner plays it in an isolated sandbox, a verified score lands on the leaderboard."""
    import base64
    import hashlib
    import io
    import tarfile
    path = os.path.abspath(args.bot)
    if not os.path.exists(path):
        print(f"sharingan: no such file: {args.bot}", file=sys.stderr)
        return 2
    # Check everything we can before the first request: registering a bot spends the per-address registration cap.
    if not MIN_HANDS <= args.hands <= MAX_HANDS:
        print(f"sharingan: --hands must be between {MIN_HANDS:,} and {MAX_HANDS:,}, got {args.hands:,}", file=sys.stderr)
        return 2
    owner = _owner(args)
    if owner is None:
        return 2
    entry = args.entry or "bot.py"
    if os.path.isdir(path) and not os.path.isfile(os.path.join(path, entry)):
        print(f"sharingan: {args.bot} has no {entry} (pass --entry FILE)", file=sys.stderr)
        return 2
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tar:
        if os.path.isdir(path):
            tar.add(path, arcname=".", filter=lambda ti: None if "__pycache__" in ti.name or ".git" in ti.name else ti)
        else:
            entry = os.path.basename(path)
            tar.add(path, arcname=entry)
    sha = hashlib.sha256(buf.getvalue()).hexdigest()   # the server hashes the same bytes and shows it as artifact_sha256
    api = ArenaClient(args.server)
    token = args.token
    if not token:
        name = _bot_name_for(path)
        try:
            token = bot_token(api, name)
        except ApiException as e:
            print(f"sharingan: could not register: {e}", file=sys.stderr)
            return 1
        except OSError as e:
            return _unreachable(args.server, e)
    body = {"botName": os.path.splitext(os.path.basename(path.rstrip("/")))[0][:48] or "bot", "language": "python", "entry": entry,
            "track": args.track, "hands": args.hands, "idempotencyKey": args.key or f"cli-{int(time.time() * 1000)}",
            "artifactTgzBase64": base64.b64encode(buf.getvalue()).decode()}
    if args.lanes:
        body["lanes"] = args.lanes
    if args.requirements:
        body["requirementsFile"] = args.requirements
    body.update(owner)
    api.timeout = 120.0
    try:
        job = api.request("POST", "/v1/jobs", body, token=token)
    except ApiException as e:
        if e.code == "competition_closed":
            print(f"sharingan: submit refused: {e.message}", file=sys.stderr)
            print("sharingan: the sharingan-1 track takes no new public submits; ladder-v1 is still open (--track ladder-v1)", file=sys.stderr)
        else:
            print(f"sharingan: submit refused: {e.status} {e.code}: {e.message}", file=sys.stderr)
        return 1
    except OSError as e:
        return _unreachable(args.server, e)
    _save_owner(owner)
    jid = job["id"]
    print(f"sharingan: job {jid} accepted ({job.get('hands') or args.hands} hands on {args.track}; "
          f"upload sha256 {(job.get('artifact_sha256') or sha)[:12]})", file=sys.stderr, flush=True)
    t0, last = time.time(), None
    api.timeout = 30.0
    while True:
        try:
            j = api.request("GET", f"/v1/jobs/{jid}", token=token)
        except (ApiException, OSError):
            time.sleep(3)
            continue
        line = f"{j['status']} {j.get('hands_done', 0)}/{j.get('hands_total', 0)} hands"
        if line != last and not args.quiet:
            print(f"sharingan: [{int(time.time() - t0)}s] {line}", file=sys.stderr, flush=True)
            last = line
        if j["status"] in ("finished", "failed", "aborted") and j.get("settled", True):
            break
        if args.timeout and time.time() - t0 > args.timeout:
            print(f"sharingan: timed out waiting; job {jid} keeps running (GET /v1/jobs/{jid})", file=sys.stderr)
            return 1
        time.sleep(2)
    if j["status"] != "finished":
        print(f"sharingan: job {j['status']}: {j.get('error')}", file=sys.stderr)
        if j.get("bot_log_tail"):
            print(j["bot_log_tail"][-1500:], file=sys.stderr)
        return 1
    short = j["verified"] and j["hands_done"] < RANKED_HANDS   # scores, but a short run never ranks
    tag = (f"VERIFIED, NOT RANKED: under {RANKED_HANDS:,} hands" if short else "VERIFIED") if j["verified"] else "unverified"
    print(f"{j['bot_name']}: {j['mbb_per_game']:+.1f} +- {j['ci95_mbb']:.1f} mbb/game over {j['hands_done']} hands on {j['track']} "
          f"[{tag}, hosted job {jid}]")
    _warn_bot_trouble(j)
    if short:
        print(f"sharingan: not ranked: the leaderboard counts verified runs of {RANKED_HANDS:,} hands, this one had {j['hands_done']:,}; "
              "submit without --hands for a ranked run", file=sys.stderr)
    else:
        print(f"leaderboard: {leaderboard_url(args.server, args.track)}")
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
    sm = sub.add_parser("submit", help="hosted job: upload the bot, the server plays it in an isolated sandbox, prints a VERIFIED score")
    sm.add_argument("bot", help="mybot.py (or a directory with --entry)")
    sm.add_argument("--server", default=os.environ.get("SHARINGAN_SERVER", DEFAULT_SERVER))
    sm.add_argument("--token", default=os.environ.get("SHARINGAN_TOKEN"), help="your arena bot token (default: the one saved for this bot name, registering it once)")
    sm.add_argument("--track", choices=TRACKS, default=DEFAULT_TRACK)
    sm.add_argument("--hands", type=int, default=20000,
                    help=f"{MIN_HANDS} to {MAX_HANDS}; the default 20000 becomes 20004 (whole deals); verified runs of 20,004 hands or more rank, shorter ones are for testing")
    sm.add_argument("--lanes", type=int, default=0)
    sm.add_argument("--entry", default=None, help="entry file when BOT is a directory (default bot.py)")
    sm.add_argument("--requirements", default=None, help="requirements.txt inside the bot directory (installed in a short network window)")
    sm.add_argument("--contact", default=os.environ.get("SHARINGAN_CONTACT"), help="required: your email, so the organizers can reach you (competition prizes); never shown publicly; saved after the first submit")
    sm.add_argument("--profile", default=os.environ.get("SHARINGAN_PROFILE"), help="required: link to your X, LinkedIn, GitHub or personal page (https://x.com/you); never shown publicly; saved after the first submit")
    sm.add_argument("--key", default=None, help="idempotency key (resubmitting the same key returns the same job)")
    sm.add_argument("--timeout", type=float, default=0)
    sm.add_argument("--quiet", action="store_true")
    tk = sub.add_parser("token", help="print the saved bot token (for curl: $(sharingan token mybot.py))")
    tk.add_argument("bot", nargs="?", default=None, help="mybot.py (named after the file) or a bot name; omit when only one bot is saved for the server")
    tk.add_argument("--server", default=os.environ.get("SHARINGAN_SERVER", DEFAULT_SERVER))
    pp = sub.add_parser("pipe", help="serve the bot over stdin/stdout (pipe protocol) for `sharingan-arena run --bot \"sharingan pipe mybot.py\"`")
    pp.add_argument("bot")
    b = sub.add_parser("benchmark", help="run a benchmark track (ladder-v1 | sharingan-1) and print mbb/game ± CI")
    b.add_argument("bot")
    b.add_argument("--track", choices=TRACKS, default=DEFAULT_TRACK)
    b.add_argument("--hands", type=int, default=None, help="hands (server default 20000; rounded up to a multiple of 6)")
    b.add_argument("--lanes", type=int, default=None, help="parallel lanes (bot must key state by table_id/hand_id)")
    b.add_argument("--server", default=os.environ.get("SHARINGAN_SERVER", DEFAULT_SERVER))
    b.add_argument("--token", default=os.environ.get("SHARINGAN_TOKEN"), help="use this bot token (default: the one saved for this bot name, registering it once)")
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
        if a.cmd == "submit":
            return _submit(a)
        if a.cmd == "token":
            return _token(a)
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
