"""In-process mock arena implementing just enough of docs/TABLE_API.md to test bots.

REST (stdlib http.server thread) + WebSocket /v1/play (websockets). Each hand is scripted:
the bot sits in seat 0 (UTG) and gets a preflop `your_turn` (facing the BB), then, unless it folds,
a flop `your_turn` (first to act, checking is free). Features: idempotent `act`, `stale_decision`,
`illegal_action`/`bad_amount` errors that re-send the same `your_turn`, `table_seq`/`seq` numbering,
`hello{resume}` -> `table_snapshot` + re-sent pending `your_turn`, optional forced disconnects.

    async with MockArena(hands=3) as arena:
        ...Runner(bot, arena.ws_url, token).run()
"""
from __future__ import annotations

import asyncio
import json
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Dict, List, Optional, Set
from urllib.parse import parse_qs, urlparse

SB, BB, STACK = 50, 100, 10000
HOLE = ["Ah", "Kd"]
FLOP = ["7c", "8d", "2s"]


def _now() -> int:
    return int(time.time() * 1000)


class MockArena:
    def __init__(self, hands: int = 3, drop_on_turns: Optional[Set[int]] = None, decision_ms: int = 5000,
                 emit_gap_at: Optional[int] = None, benchmarks: bool = False,
                 bench_result: tuple = (42.5, 12.0)):
        self.hands = hands
        self.benchmarks = benchmarks          # serve /v1/benchmarks + /runs (scripted run; progress over the bot's socket)
        self.bench_result = bench_result      # (mbb_per_game, ci95_mbb) of a finished run
        self.runs: Dict[str, dict] = {}
        self.run_requests: List[dict] = []    # bodies of POST /v1/benchmarks/{track}/runs
        self._sockets: Dict[str, Any] = {}    # player_id -> live bot websocket
        self._loop: Any = None
        self.drop_on_turns = set(drop_on_turns or ())  # 1-based counts of your_turn sends after which to drop the socket
        self.decision_ms = decision_ms
        self.emit_gap_at = emit_gap_at  # skip a table_seq number once (tests gap resync)
        self.players: Dict[str, dict] = {}
        self.tokens: Dict[str, str] = {}
        self.tables: Dict[str, dict] = {}
        self.received: List[dict] = []        # every envelope the client sent
        self.acts: List[dict] = []            # accepted act data
        self.errors_sent: List[str] = []
        self.hand_ends: List[dict] = []
        self.turns_sent = 0
        self.connections = 0
        self.hand_no = 0
        self.phase = "idle"                   # idle | preflop | flop | finished
        self.pending: Optional[dict] = None   # your_turn data awaiting an answer
        self.answered: Set[str] = set()
        self.table_id = "t1"
        self.table_seq = 0
        self.seq = 0
        self._x = 0                           # bot's street contribution chosen preflop
        self._gap_done = False
        self._ws_server: Any = None
        self._http: Optional[ThreadingHTTPServer] = None
        self.http_url = ""
        self.url = ""                         # REST + WebSocket on ONE port (what `--server` takes)
        self.ws_url = ""
        self.finished: Optional[asyncio.Event] = None  # set in __aenter__

    # ------------------------------------------------------------------ lifecycle
    async def __aenter__(self) -> "MockArena":
        import websockets
        self.finished = asyncio.Event()
        self._loop = asyncio.get_running_loop()
        self._ws_server = await websockets.serve(self._ws_handler, "127.0.0.1", 0)
        port = self._ws_server.sockets[0].getsockname()[1]
        self.ws_url = f"ws://127.0.0.1:{port}/v1/play"
        self._start_http()
        self._front = await asyncio.start_server(self._front_handler, "127.0.0.1", 0)
        self.url = f"http://127.0.0.1:{self._front.sockets[0].getsockname()[1]}"
        return self

    async def _front_handler(self, r: asyncio.StreamReader, w: asyncio.StreamWriter) -> None:
        """Single-port front (like the real server): WebSocket upgrades go to the ws server, the rest to REST."""
        try:
            head = await r.readuntil(b"\r\n\r\n")
            ws_port = self._ws_server.sockets[0].getsockname()[1]
            port = ws_port if b"upgrade: websocket" in head.lower() else self._http.server_address[1]
            ur, uw = await asyncio.open_connection("127.0.0.1", port)
            uw.write(head)

            async def pipe(src: asyncio.StreamReader, dst: asyncio.StreamWriter) -> None:
                try:
                    while chunk := await src.read(65536):
                        dst.write(chunk)
                        await dst.drain()
                except (ConnectionError, asyncio.CancelledError):
                    pass
                finally:
                    dst.close()
            await asyncio.gather(pipe(r, uw), pipe(ur, w))
        except (asyncio.IncompleteReadError, ConnectionError):
            pass
        finally:
            w.close()

    async def __aexit__(self, *exc: Any) -> None:
        self._front.close()
        self._ws_server.close()
        await self._ws_server.wait_closed()
        if self._http:
            self._http.shutdown()
            self._http.server_close()

    # ------------------------------------------------------------------ REST
    def _start_http(self) -> None:
        arena = self

        class H(BaseHTTPRequestHandler):
            def _send(self, status: int, obj: Any) -> None:
                b = json.dumps(obj).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(b)))
                self.end_headers()
                self.wfile.write(b)

            def _err(self, status: int, code: str, msg: str = "") -> None:
                self._send(status, {"error": {"code": code, "message": msg, "details": {}}})

            def _handle(self, method: str) -> None:
                n = int(self.headers.get("Content-Length") or 0)
                body = json.loads(self.rfile.read(n)) if n else {}
                path = urlparse(self.path).path
                parts = [p for p in path.split("/") if p]
                if method == "POST" and path == "/v1/players":
                    pid = "p-" + uuid.uuid4().hex[:8]
                    tok = "tok-" + uuid.uuid4().hex
                    rec = {"player_id": pid, "name": body.get("name", ""), "kind": body.get("kind", "bot"),
                           "created_at": "2026-01-01T00:00:00Z"}
                    arena.players[pid] = rec
                    arena.tokens[tok] = pid
                    return self._send(201, {**rec, "token": tok})
                if arena.benchmarks and method == "GET" and path == "/v1/benchmarks":
                    return self._send(200, [{"track": t, "version": 1, "available": True, "default_hands": 20000,
                                             "panel": [], "seed": f"bench:{t}:v1"} for t in ("ladder-v1", "sharingan-1")])
                auth = (self.headers.get("Authorization") or "").replace("Bearer ", "")
                if auth not in arena.tokens:
                    return self._err(401, "unauthorized", "bad token")
                if arena.benchmarks and parts[:2] == ["v1", "benchmarks"]:
                    pid = arena.tokens[auth]
                    if len(parts) == 4 and parts[3] == "runs" and method == "POST":
                        if pid not in arena._sockets:
                            return self._err(409, "not_connected", "connect to /v1/play first")
                        arena.run_requests.append(body)
                        return self._send(201, arena._start_run(parts[2], pid, int(body.get("hands") or 20000)))
                    if len(parts) == 5 and parts[3] == "runs" and method == "GET":
                        r = arena.runs.get(parts[4])
                        return self._send(200, dict(r)) if r else self._err(404, "not_found")
                if method == "POST" and path == "/v1/tables":
                    tid = "t-" + uuid.uuid4().hex[:6]
                    arena.tables[tid] = {"table_id": tid, "config": body, "status": "waiting", "button_seat": None,
                                         "hand_id": None, "table_seq": 0,
                                         "seats": [{"seat": i, "player_id": None, "name": None, "stack": 0, "status": "empty"}
                                                   for i in range(6)]}
                    return self._send(201, arena.tables[tid])
                if method == "POST" and path == "/v1/matches":
                    arena.matches = getattr(arena, "matches", {})
                    mid = "m-" + uuid.uuid4().hex[:6]
                    arena.matches[mid] = {"match_id": mid, "participants": body["participants"], "hands_planned": body["hands_planned"],
                                          "status": "scheduled", "hands_played": 0, "duplicate": body.get("duplicate"), "seed": body.get("seed")}
                    return self._send(201, arena.matches[mid])
                if len(parts) == 4 and parts[:2] == ["v1", "matches"] and parts[3] in ("start", "pause", "abort"):
                    m = arena.matches[parts[2]]
                    m["status"] = "running"
                    return self._send(200, m)
                if len(parts) == 3 and parts[:2] == ["v1", "matches"] and method == "GET":
                    m = arena.matches[parts[2]]
                    m.update(status="finished", hands_played=m["hands_planned"])  # instantly done (CLI tests)
                    me = m["participants"][0]
                    return self._send(200, {**m, "standings": [{"player_id": me, "name": "x", "hands": m["hands_played"], "net_chips": 1,
                        "bb_per_100": 5.0, "mbb_per_hand": 50.0, "ci95_bb_per_100": 2.5, "duplicate_bb_per_100": 4.0}]})
                if len(parts) == 3 and parts[:2] == ["v1", "tables"] and method == "GET":
                    t = arena.tables.get(parts[2])
                    return self._send(200, t) if t else self._err(404, "not_found")
                if len(parts) == 5 and parts[:2] == ["v1", "tables"] and parts[3] == "seats":
                    t = arena.tables.get(parts[2])
                    if not t:
                        return self._err(404, "not_found")
                    seat = t["seats"][int(parts[4])]
                    if method == "POST":
                        if seat["player_id"]:
                            return self._err(409, "seat_taken")
                        pl = arena.players[body["player_id"]]
                        seat.update(player_id=pl["player_id"], name=pl["name"], stack=STACK, status="seated")
                        return self._send(200, seat)
                    if method == "DELETE":
                        seat.update(player_id=None, name=None, stack=0, status="empty")
                        return self._send(200, seat)
                self._err(404, "not_found")

            def do_GET(self) -> None: self._handle("GET")      # noqa: E701,N802
            def do_POST(self) -> None: self._handle("POST")    # noqa: E701,N802
            def do_DELETE(self) -> None: self._handle("DELETE")  # noqa: E701,N802
            def log_message(self, *a: Any) -> None: pass       # noqa: E701

        self._http = ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.http_url = f"http://127.0.0.1:{self._http.server_address[1]}"
        threading.Thread(target=self._http.serve_forever, daemon=True).start()

    # ------------------------------------------------------------------ benchmark runs
    def _start_run(self, track: str, player_id: str, hands: int) -> dict:
        total = (hands + 5) // 6 * 6
        rid = "r-" + uuid.uuid4().hex[:6]
        run = {"run_id": rid, "track": track, "player_id": player_id, "status": "running", "hands_planned": total,
               "hands_total": total, "hands_done": 0, "verified": False, "mbb_per_game": None, "ci95_mbb": None,
               "method": None, "running_mbb_per_game": None, "running_ci95_mbb": None}
        self.runs[rid] = run
        asyncio.run_coroutine_threadsafe(self._play_run(run), self._loop)
        return dict(run)

    async def _play_run(self, run: dict) -> None:
        """Scripted run: benchmark_progress at 25/50/75% then a final 100% (status finished)."""
        total, (mbb, ci) = run["hands_total"], self.bench_result
        for step in (1, 2, 3, 4):
            await asyncio.sleep(0.1)
            last = step == 4
            done = total if last else total * step // 4
            if last:
                run.update(status="finished", mbb_per_game=mbb, ci95_mbb=ci, method="aivat")
            run["hands_done"] = done
            run.update(running_mbb_per_game=mbb * done / total, running_ci95_mbb=ci)
            ws = self._sockets.get(run["player_id"])
            if ws is not None:
                data = {"run_id": run["run_id"], "track": run["track"], "match_id": "m-bench", "status": run["status"],
                        "hands_done": done, "hands_total": total, "mbb_per_game": run["mbb_per_game"] if last else None,
                        "ci95_mbb": ci if last else None, "method": run["method"], "verified": False}
                try:
                    await ws.send(json.dumps({"v": 1, "type": "benchmark_progress", "t_ms": _now(), "data": data}))
                except Exception:
                    pass

    # ------------------------------------------------------------------ WS
    def _env(self, type_: str, data: dict, hand: bool = True, with_seq: bool = True) -> str:
        self.table_seq += 1
        if self.emit_gap_at and self.table_seq == self.emit_gap_at and not self._gap_done:
            self._gap_done = True
            self.table_seq += 1  # simulate a lost message
        env: Dict[str, Any] = {"v": 1, "type": type_, "table_id": self.table_id, "table_seq": self.table_seq,
                               "t_ms": _now(), "data": data}
        if hand and self.hand_no:
            env["hand_id"] = f"h-{self.hand_no}"
            if with_seq:
                self.seq += 1
                env["seq"] = self.seq
                data.setdefault("seq", self.seq)
        return json.dumps(env)

    async def _ws_handler(self, ws: Any) -> None:
        path = getattr(getattr(ws, "request", None), "path", None) or getattr(ws, "path", "")
        token = (parse_qs(urlparse(path).query).get("token") or [""])[0]
        if token not in self.tokens and not token.startswith("any"):
            await ws.close(4401, "unauthorized")
            return
        self.connections += 1
        self.player_id = self.tokens.get(token, "p0")
        self._sockets[self.player_id] = ws
        try:
            async for raw in ws:
                msg = json.loads(raw)
                self.received.append(msg)
                t = msg.get("type")
                if t == "hello":
                    await self._on_hello(ws, msg)
                elif t == "ping":
                    await ws.send(json.dumps({"v": 1, "type": "pong", "t_ms": _now(), "data": {}}))
                elif t == "act":
                    await self._on_act(ws, msg["data"])
        except Exception as e:  # connection closed etc.
            if self._sockets.get(self.player_id) is ws:
                del self._sockets[self.player_id]
            if type(e).__name__ not in ("ConnectionClosedOK", "ConnectionClosedError", "ConnectionClosed"):
                raise

    async def _on_hello(self, ws: Any, msg: dict) -> None:
        await ws.send(json.dumps({"v": 1, "type": "welcome", "t_ms": _now(),
                                  "data": {"player_id": self.player_id, "name": "bot", "server_time": _now(),
                                           "tables": [self.table_id], "protocol": 1}}))
        seats = [{"seat": i, "player_id": self.player_id if i == 0 else f"other{i}", "name": f"s{i}",
                  "stack": STACK, "status": "seated"} for i in range(6)]
        snap = {"table_id": self.table_id, "config": {"small_blind": SB, "big_blind": BB, "starting_stack": STACK},
                "status": "running", "seats": seats, "button_seat": 3, "hand_id": f"h-{self.hand_no}" if self.hand_no else None,
                "table_seq": self.table_seq, "hand": None}
        await ws.send(json.dumps({"v": 1, "type": "table_snapshot", "table_id": self.table_id,
                                  "table_seq": self.table_seq, "t_ms": _now(), "data": snap}))
        if self.phase == "idle":
            await self._start_hand(ws)
        elif self.pending:
            await self._send_turn(ws)  # same decision_id (conformance #7)

    def _players(self, street_contrib: Dict[int, int], status: Optional[Dict[int, str]] = None) -> List[dict]:
        out = []
        for s in range(6):
            c = street_contrib.get(s, 0)
            out.append({"seat": s, "name": f"s{s}", "stack_start": STACK, "stack": STACK - c, "street_contrib": c,
                        "hand_contrib": c, "status": (status or {}).get(s, "active")})
        return out

    def _action_ev(self, seat: int, type_: str, to: int, added: int, stack: int, pot: int, street: str,
                   forced: bool = False, nxt: Optional[int] = None) -> dict:
        self.seq += 1
        return {"hand_id": f"h-{self.hand_no}", "seq": self.seq, "action_id": f"a-{self.hand_no}-{self.seq}",
                "street": street, "seat": seat, "type": type_, "to": to, "added": added, "all_in": False,
                "forced": forced, "timeout": False, "stack": stack, "pot": pot,
                "current_bet": to if type_ != "fold" else BB, "min_raise_to": 2 * BB, "next_to_act": nxt}

    async def _start_hand(self, ws: Any) -> None:
        self.hand_no += 1
        self.seq = 0
        self.phase = "preflop"
        self.answered.clear()
        hs = {"hand_id": f"h-{self.hand_no}", "hand_no": self.hand_no, "match_id": "m1", "deal_no": None,
              "rotation": None, "button_seat": 3, "sb_seat": 4, "bb_seat": 5, "small_blind": SB, "big_blind": BB,
              "ante": 0, "players": [{"seat": i, "player_id": self.player_id if i == 0 else f"other{i}",
                                      "name": f"s{i}", "stack_start": STACK, "in_hand": True} for i in range(6)],
              "deck_commitment": "00" * 32}
        await ws.send(self._env("hand_start", hs, with_seq=False))
        await ws.send(self._env("hole_cards", {"hand_id": f"h-{self.hand_no}", "seat": 0, "cards": HOLE}, with_seq=False))
        self.history = [self._action_ev(4, "post_sb", SB, SB, STACK - SB, SB, "preflop", True, 5),
                        self._action_ev(5, "post_bb", BB, BB, STACK - BB, SB + BB, "preflop", True, 0)]
        for ev in self.history:
            await ws.send(self._env("action", dict(ev), with_seq=False))
        self.pending = self._turn("preflop", [], {4: SB, 5: BB}, pot=SB + BB, call={"to": BB, "added": BB, "all_in": False},
                                  check=False, raise_={"min_to": 2 * BB, "max_to": STACK})
        await self._send_turn(ws)

    def _turn(self, street: str, board: List[str], contrib: Dict[int, int], pot: int, call: Optional[dict],
              check: bool, raise_: dict) -> dict:
        self.seq += 1
        return {"hand_id": f"h-{self.hand_no}", "seq": self.seq, "decision_id": f"d-{self.hand_no}-{self.seq}",
                "seat": 0, "street": street, "hole_cards": HOLE, "board": board, "history": list(self.history),
                "players": self._players(contrib), "button_seat": 3, "small_blind": SB, "big_blind": BB, "pot": pot,
                "pots": [{"amount": pot, "eligible_seats": [0, 4, 5]}],
                "legal": {"fold": True, "check": check, "call": call, "raise": raise_},
                "deadline_ms": _now() + self.decision_ms, "time_bank_ms": 30000}

    async def _send_turn(self, ws: Any) -> None:
        assert self.pending is not None
        self.pending["deadline_ms"] = _now() + self.decision_ms
        self.turns_sent += 1
        env = json.loads(self._env("your_turn", dict(self.pending), with_seq=False))
        env["seq"] = self.pending["seq"]
        await ws.send(json.dumps(env))
        if self.turns_sent in self.drop_on_turns:
            self.drop_on_turns.discard(self.turns_sent)
            await ws.close()

    async def _err(self, ws: Any, code: str, msg: str, decision_id: Optional[str] = None, details: Optional[dict] = None) -> None:
        self.errors_sent.append(code)
        d: Dict[str, Any] = {"code": code, "message": msg, "details": details or {}}
        if decision_id:
            d["decision_id"] = decision_id
        await ws.send(self._env("error", d, hand=False))

    async def _on_act(self, ws: Any, d: dict) -> None:
        p = self.pending
        if p is None or d.get("hand_id") != p["hand_id"]:
            return await self._err(ws, "not_your_turn", "no pending turn")
        if d.get("decision_id") in self.answered:
            return  # idempotent duplicate
        if d.get("decision_id") != p["decision_id"]:
            return await self._err(ws, "stale_decision", "stale", d.get("decision_id"))
        t, to, legal = d.get("type"), d.get("to"), p["legal"]
        ok = (t == "fold" and legal["fold"]) or (t == "check" and legal["check"]) or (t == "call" and legal["call"]) \
            or (t in ("raise", "bet") and legal["raise"])
        if not ok:
            await self._err(ws, "illegal_action", f"{t} not legal", p["decision_id"])
            return await self._send_turn(ws)
        if t in ("raise", "bet"):
            r = legal["raise"]
            if not isinstance(to, int) or not (r["min_to"] <= to <= r["max_to"]):
                await self._err(ws, "bad_amount", "to out of range", p["decision_id"], {"min_to": r["min_to"], "max_to": r["max_to"]})
                return await self._send_turn(ws)
        self.answered.add(p["decision_id"])
        self.acts.append(d)
        self.pending = None
        street = p["street"]
        if t == "fold" and legal["check"]:
            t = "check"  # converted (§6.6 preferred behaviour)
        contrib_to = {"fold": 0, "check": 0, "call": legal["call"]["to"] if legal["call"] else 0}.get(t, to)
        added = contrib_to if street == "preflop" else contrib_to
        ev = self._action_ev(0, t, contrib_to, added, STACK - contrib_to, p["pot"] + added, street, nxt=None)
        self.history.append(ev)
        await ws.send(self._env("action", dict(ev), with_seq=False))
        if street == "preflop":
            if t == "fold":
                return await self._end_hand(ws, [4, 5], "fold", bot_net=0, bb_wins=True)
            self._x = contrib_to
            pot = 2 * self._x + SB  # SB folds, BB calls
            board = FLOP
            await ws.send(self._env("street", {"hand_id": f"h-{self.hand_no}", "street": "flop", "board": board,
                                               "new_cards": board, "pot": pot, "pots": [{"amount": pot, "eligible_seats": [0, 5]}]},
                                    with_seq=False))
            self.phase = "flop"
            self.pending = self._turn("flop", board, {}, pot=pot, call=None, check=True,
                                      raise_={"min_to": BB, "max_to": STACK - self._x})
            await self._send_turn(ws)
        else:
            if t in ("raise", "bet"):  # BB folds
                return await self._end_hand(ws, [5], "fold", bot_net=self._x + SB, bb_wins=False)
            win = self.hand_no % 2 == 1
            await self._end_hand(ws, [0, 5], "showdown", bot_net=self._x if win else -self._x, bb_wins=not win)

    async def _end_hand(self, ws: Any, seats: List[int], ended_by: str, bot_net: int, bb_wins: bool) -> None:
        # nets: SB always -50; bot + bb absorb the rest so that sum == 0
        bb_net = SB - bot_net
        results = [{"seat": 0, "player_id": self.player_id, "net": bot_net, "stack_end": STACK + bot_net},
                   {"seat": 4, "player_id": "other4", "net": -SB, "stack_end": STACK - SB},
                   {"seat": 5, "player_id": "other5", "net": bb_net, "stack_end": STACK + bb_net}]
        data = {"hand_id": f"h-{self.hand_no}", "board": FLOP + ["3h", "9c"],
                "showdown": [{"seat": 0, "cards": HOLE, "shown": ended_by == "showdown", "hand_rank": "high card", "best5": []}]
                if ended_by == "showdown" else [],
                "hole_cards": {str(i): (HOLE if i == 0 else ["2c", "3d"]) for i in range(6)},
                "pots": [{"amount": 2 * self._x + SB, "eligible_seats": [0, 5], "winners": [0 if bot_net > 0 else 5],
                          "share": [2 * self._x + SB]}],
                "results": results, "deck_reveal": {"deck": [], "salt": "x"}, "ended_by": ended_by}
        self.hand_ends.append(data)
        await ws.send(self._env("hand_end", data, with_seq=True))
        self.phase = "idle"
        if self.hand_no >= self.hands:
            self.phase = "finished"
            if self.finished:
                self.finished.set()
            return
        await self._start_hand(ws)
