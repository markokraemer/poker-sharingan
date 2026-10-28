"""REST client (`ArenaClient`, stdlib urllib) and WebSocket runner (`Runner`, uses `websockets`)."""
from __future__ import annotations

import asyncio
import inspect
import json
import logging
import time
import urllib.error
import urllib.request
from typing import Any, Callable, Dict, List, Optional, Union
from urllib.parse import urlencode, urlparse, urlunparse

from .bot import Bot
from .helpers import legalize, safe_action
from .models import (Action, ActionEvent, ApiError, HandEnd, HandStart, PlayerInfo, Seat, StreetEvent,
                     Table, YourTurn)

log = logging.getLogger("sharingan")

PROTOCOL_VERSION = 1
CLIENT_NAME = "sharingan-py/0.1.0"


class ApiException(Exception):
    """HTTP-level error from the REST API (`{"error": {code, message, details}}`)."""

    def __init__(self, status: int, code: str, message: str = "", details: Optional[dict] = None):
        super().__init__(f"{status} {code}: {message}")
        self.status, self.code, self.message, self.details = status, code, message, details or {}


# ----------------------------------------------------------------------------- REST

class ArenaClient:
    """Thin REST client for /v1. Synchronous (stdlib only); use `asyncio.to_thread` if you need it in async code.

    `base_url` may be http(s)://host[:port] (or ws(s)://, which is mapped to http(s)).
    """

    def __init__(self, base_url: str, token: Optional[str] = None, timeout: float = 10.0):
        u = urlparse(base_url)
        scheme = {"ws": "http", "wss": "https"}.get(u.scheme, u.scheme or "http")
        self.base_url = urlunparse((scheme, u.netloc, u.path.rstrip("/"), "", "", ""))
        self.token = token
        self.timeout = timeout

    # -- low level
    def request(self, method: str, path: str, body: Any = None, token: Optional[str] = None,
                query: Optional[dict] = None) -> Any:
        url = self.base_url + path
        if query:
            url += "?" + urlencode({k: v for k, v in query.items() if v is not None})
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(url, data=data, method=method)
        req.add_header("Accept", "application/json")
        if data is not None:
            req.add_header("Content-Type", "application/json")
        tok = token or self.token
        if tok:
            req.add_header("Authorization", f"Bearer {tok}")
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as r:
                raw = r.read()
        except urllib.error.HTTPError as e:
            raw = e.read()
            try:
                err = json.loads(raw)["error"]
            except Exception:
                err = {"code": "internal", "message": raw.decode(errors="replace")[:200]}
            raise ApiException(e.code, err.get("code", "internal"), err.get("message", ""), err.get("details")) from None
        return json.loads(raw) if raw else None

    # -- players
    def register_bot(self, name: str, kind: str = "bot", set_token: bool = True) -> PlayerInfo:
        """POST /v1/players. The returned `token` is shown once; by default it becomes this client's token."""
        p = PlayerInfo.from_dict(self.request("POST", "/v1/players", {"name": name, "kind": kind}))
        if set_token and p.token:
            self.token = p.token
        return p

    def get_player(self, player_id: str) -> PlayerInfo:
        return PlayerInfo.from_dict(self.request("GET", f"/v1/players/{player_id}"))

    # -- tables
    def create_table(self, **config: Any) -> Table:
        """POST /v1/tables. Defaults: 6-max NLHE 50/100, 10000 stack, reset mode."""
        cfg = {"variant": "nlhe", "max_seats": 6, "small_blind": 50, "big_blind": 100, "ante": 0, "straddle": False,
               "starting_stack": 10000, "stack_mode": "reset", "decision_time_ms": 5000, "time_bank_ms": 30000,
               "timeout_action": "check_fold", "rake": {"percent": 0, "cap": 0}, "min_players_to_deal": 6,
               "odd_chip_rule": "first_left_of_button", "show_mode": "standard"}
        cfg.update(config)
        return Table.from_dict(self.request("POST", "/v1/tables", cfg))

    def get_table(self, table_id: str) -> Table:
        return Table.from_dict(self.request("GET", f"/v1/tables/{table_id}"))

    def join_table(self, table_id: str, seat: int, player_id: str, token: Optional[str] = None) -> Seat:
        """POST /v1/tables/{id}/seats/{seat} (sit down)."""
        return Seat.from_dict(self.request("POST", f"/v1/tables/{table_id}/seats/{seat}",
                                           {"player_id": player_id}, token=token))

    def leave_table(self, table_id: str, seat: int) -> None:
        self.request("DELETE", f"/v1/tables/{table_id}/seats/{seat}")

    # -- matches / hands
    def create_match(self, table_config: dict, participants: List[str], hands_planned: int,
                     duplicate: Optional[dict] = None, seed: Optional[str] = None) -> dict:
        body: Dict[str, Any] = {"table_config": table_config, "participants": participants, "hands_planned": hands_planned}
        if duplicate is not None:
            body["duplicate"] = duplicate
        if seed is not None:
            body["seed"] = seed
        return self.request("POST", "/v1/matches", body)

    def match_action(self, match_id: str, action: str) -> dict:
        """action in start | pause | abort"""
        return self.request("POST", f"/v1/matches/{match_id}/{action}")

    def get_match(self, match_id: str) -> dict:
        return self.request("GET", f"/v1/matches/{match_id}")

    def get_hands(self, match_id: str, after_hand_no: Optional[int] = None, limit: int = 500) -> List[dict]:
        return self.request("GET", f"/v1/matches/{match_id}/hands",
                            query={"after_hand_no": after_hand_no, "limit": limit})

    def get_hand(self, hand_id: str) -> dict:
        return self.request("GET", f"/v1/hands/{hand_id}")

    def get_hand_events(self, hand_id: str) -> List[dict]:
        return self.request("GET", f"/v1/hands/{hand_id}/events")


def ws_url(url: str, token: Optional[str] = None) -> str:
    """Accepts http(s)://host, ws(s)://host or a full ws(s)://host/v1/play URL; appends /v1/play and ?token=."""
    u = urlparse(url)
    scheme = {"http": "ws", "https": "wss"}.get(u.scheme, u.scheme or "ws")
    path = u.path.rstrip("/")
    if not path.endswith("/v1/play"):
        path += "/v1/play"
    query = u.query
    if token and "token=" not in query:
        query = (query + "&" if query else "") + urlencode({"token": token})
    return urlunparse((scheme, u.netloc, path, "", query, ""))


# ----------------------------------------------------------------------------- WebSocket runner

async def _maybe_await(v: Any) -> Any:
    return await v if inspect.isawaitable(v) else v


class Runner:
    """Connects a `Bot` to /v1/play: hello/resume, keep-alive, dedupe, gap resync, reconnect with backoff,
    deadline-aware `act` with a check/fold fallback.

    Typical use: `run(bot, url, token)`  (blocking)  or  `await Runner(bot, url, token).run()`.
    """

    def __init__(self, bot: Bot, url: str, token: str, *, reconnect: bool = True, max_backoff: float = 10.0,
                 ping_interval: float = 15.0, safety_margin_ms: int = 150, max_hands: Optional[int] = None,
                 on_message: Optional[Callable[[dict], Any]] = None, client_name: str = CLIENT_NAME):
        self.bot, self.token = bot, token
        self.url = ws_url(url, token)
        self.reconnect, self.max_backoff = reconnect, max_backoff
        self.ping_interval, self.safety_margin_ms = ping_interval, safety_margin_ms
        self.max_hands = max_hands
        self.on_message = on_message
        self.client_name = client_name
        # state
        self.welcome: Optional[dict] = None
        self.player_id: Optional[str] = None
        self.tables: Dict[str, Table] = {}
        self.last_table_seq: Dict[str, int] = {}
        self._hand_seq: Dict[str, int] = {}
        self.hands_played = 0
        self.net_chips = 0
        self.connects = 0
        self.errors: List[ApiError] = []
        self._answered: set = set()
        self._tasks: set = set()
        self._ws: Any = None
        self._stop = False
        self._resyncing = False
        self._my_seats: Dict[str, int] = {}

    # -- public
    def stop(self) -> None:
        self._stop = True
        ws = self._ws
        if ws is not None:
            asyncio.ensure_future(ws.close())

    async def run(self) -> None:
        import websockets
        backoff = 0.2
        while not self._stop:
            try:
                async with websockets.connect(self.url, ping_interval=None, max_size=16 * 2**20) as ws:
                    self._ws = ws
                    self.connects += 1
                    backoff = 0.2
                    await self._session(ws)
            except (OSError, asyncio.TimeoutError, websockets.exceptions.WebSocketException) as e:
                log.warning("connection lost: %r", e)
            finally:
                self._ws = None
                await self._cancel_tasks()
            if self._stop or not self.reconnect:
                break
            await asyncio.sleep(backoff)
            backoff = min(self.max_backoff, backoff * 2)

    async def send(self, type_: str, data: dict, table_id: Optional[str] = None) -> None:
        env: Dict[str, Any] = {"v": PROTOCOL_VERSION, "type": type_, "t_ms": int(time.time() * 1000), "data": data}
        if table_id:
            env["table_id"] = table_id
        if self._ws is not None:
            await self._ws.send(json.dumps(env, separators=(",", ":")))

    async def leave(self, table_id: str) -> None:
        await self.send("leave", {"table_id": table_id}, table_id)

    # -- internals
    async def _hello(self, resume: bool) -> None:
        data: Dict[str, Any] = {"client": self.client_name}
        if resume and self.last_table_seq:
            data["resume"] = [{"table_id": t, "last_table_seq": s} for t, s in self.last_table_seq.items()]
        await self.send("hello", data)

    async def _session(self, ws: Any) -> None:
        self._answered.clear()
        self._resyncing = False
        await self._hello(resume=True)
        pinger = asyncio.ensure_future(self._ping_loop())
        try:
            async for raw in ws:
                try:
                    env = json.loads(raw)
                except ValueError:
                    log.warning("non-JSON frame ignored")
                    continue
                await self._dispatch(env)
                if self._stop:
                    return
        finally:
            pinger.cancel()

    async def _ping_loop(self) -> None:
        while True:
            await asyncio.sleep(self.ping_interval)
            try:
                await self.send("ping", {})
            except Exception:
                return

    async def _cancel_tasks(self) -> None:
        for t in list(self._tasks):
            t.cancel()
        if self._tasks:
            await asyncio.gather(*self._tasks, return_exceptions=True)
        self._tasks.clear()

    def _gap(self, env: dict) -> bool:
        """Update seq tracking. Returns True if this message is a duplicate that must be dropped."""
        tid = env.get("table_id")
        ts = env.get("table_seq")
        if tid and isinstance(ts, int):
            last = self.last_table_seq.get(tid)
            if last is not None and ts <= last and env.get("type") != "your_turn":
                return True
            if last is not None and ts > last + 1 and not self._resyncing:
                log.warning("table_seq gap %s: %s -> %s; resyncing", tid, last, ts)
                self._resyncing = True
                asyncio.ensure_future(self._hello(resume=True))
            self.last_table_seq[tid] = max(ts, last or 0)
        hid, seq = env.get("hand_id"), env.get("seq")
        if hid and isinstance(seq, int) and env.get("type") != "your_turn":
            last_h = self._hand_seq.get(hid)
            if last_h is not None and seq > last_h + 1 and not self._resyncing:
                log.warning("hand seq gap %s: %s -> %s; resyncing", hid, last_h, seq)
                self._resyncing = True
                asyncio.ensure_future(self._hello(resume=True))
            self._hand_seq[hid] = max(seq, last_h or 0)
            if len(self._hand_seq) > 64:
                self._hand_seq.pop(next(iter(self._hand_seq)))
        return False

    async def _call_hook(self, name: str, *args: Any) -> None:
        fn = getattr(self.bot, name, None)
        if fn is None:
            return
        try:
            await _maybe_await(fn(*args))
        except Exception:
            log.exception("bot.%s raised", name)

    async def _dispatch(self, env: dict) -> None:
        if self.on_message:
            await _maybe_await(self.on_message(env))
        typ = env.get("type")
        data = env.get("data") or {}
        tid = env.get("table_id") or data.get("table_id") or ""
        if typ == "pong":
            return
        if typ == "table_snapshot":
            t = Table.from_dict(data)
            self.tables[t.table_id or tid] = t
            self.last_table_seq[t.table_id or tid] = t.table_seq
            self._hand_seq.clear()
            self._resyncing = False
            self._note_seats(t)
            return
        if self._gap(env):
            return
        if typ == "welcome":
            self.welcome = data
            self.player_id = data.get("player_id")
            await self._call_hook("on_connect", data)
        elif typ == "hand_start":
            h = HandStart.from_dict(data)
            for p in h.players:
                if p.player_id and p.player_id == self.player_id:
                    self._my_seats[tid] = p.seat
            await self._call_hook("on_hand_start", h)
        elif typ == "action":
            await self._call_hook("on_action", ActionEvent.from_dict(data))
        elif typ == "street":
            await self._call_hook("on_street", StreetEvent.from_dict(data))
        elif typ == "hand_end":
            end = HandEnd.from_dict(data)
            self.hands_played += 1
            seat = self._my_seats.get(tid)
            if seat is None:
                for r in end.results:
                    if r.player_id and r.player_id == self.player_id:
                        seat = r.seat
            if seat is not None:
                self.net_chips += end.net_for(seat)
            await self._call_hook("on_hand_end", end)
            if self.max_hands is not None and self.hands_played >= self.max_hands:
                self._stop = True
                if self._ws is not None:
                    await self._ws.close()
        elif typ == "your_turn":
            self._start_decision(YourTurn.from_dict(data, table_id=tid, t_ms=int(env.get("t_ms") or 0),
                                                 hand_id=env.get("hand_id") or "", seq=int(env.get("seq") or 0)))
        elif typ == "error":
            err = ApiError.from_dict(data)
            self.errors.append(err)
            if err.decision_id:  # server will re-send your_turn with the same id; allow answering again
                self._answered.discard(err.decision_id)
            log.warning("server error %s: %s", err.code, err.message)
            await self._call_hook("on_error", err)

    def _note_seats(self, t: Table) -> None:
        for s in t.seats:
            if s.player_id and s.player_id == self.player_id:
                self._my_seats[t.table_id] = s.seat

    def _start_decision(self, state: YourTurn) -> None:
        if state.decision_id in self._answered:
            return
        self._answered.add(state.decision_id)
        task = asyncio.ensure_future(self._decide(state))
        self._tasks.add(task)
        task.add_done_callback(self._tasks.discard)

    def _budget_s(self, state: YourTurn) -> Optional[float]:
        if not state.deadline_ms:
            return None
        # Use server clock when the envelope carries it (skew-proof), else the local clock.
        ref = state.t_ms or int(time.time() * 1000)
        return max(0.0, (state.deadline_ms - ref - self.safety_margin_ms) / 1000.0)

    async def _decide(self, state: YourTurn) -> None:
        budget = self._budget_s(state)
        action: Optional[Action] = None
        try:
            if inspect.iscoroutinefunction(self.bot.act):
                action = await asyncio.wait_for(self.bot.act(state), budget)
            else:  # sync act (e.g. a blocking LLM call) runs in a thread so the deadline is enforced
                loop = asyncio.get_running_loop()
                action = await asyncio.wait_for(loop.run_in_executor(None, self.bot.act, state), budget)
        except asyncio.TimeoutError:
            log.warning("act() exceeded the decision deadline; playing check/fold")
        except asyncio.CancelledError:
            raise
        except Exception:
            log.exception("bot.act raised; playing check/fold")
        if not isinstance(action, Action):
            action = safe_action(state)
        elif not getattr(self.bot, "strict", False):
            action = legalize(state, action)
        try:
            await self.send("act", action.to_data(state.hand_id, state.decision_id), state.table_id or None)
        except Exception as e:  # connection died; the next session will be re-offered the turn
            log.warning("could not send act: %r", e)
            self._answered.discard(state.decision_id)


def run(bot: Bot, url: str, token: str, **kwargs: Any) -> Runner:
    """Blocking convenience: run `bot` until Ctrl-C (or `max_hands`). Returns the Runner (stats)."""
    r = Runner(bot, url, token, **kwargs)
    try:
        asyncio.run(r.run())
    except KeyboardInterrupt:
        pass
    return r
