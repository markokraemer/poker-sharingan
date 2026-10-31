"""Webhook mode (§11): stateless bots answer `your_turn` POSTs. stdlib http.server, no extra deps."""
from __future__ import annotations

import hashlib
import hmac
import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Optional, Tuple

from .bot import Bot
from .helpers import legalize, safe_action
from .models import Action, YourTurn


def sign(secret: str, body: bytes) -> str:
    return "sha256=" + hmac.new(secret.encode(), body, hashlib.sha256).hexdigest()


def verify(secret: str, body: bytes, header: Optional[str]) -> bool:
    return bool(header) and hmac.compare_digest(sign(secret, body), header)  # type: ignore[arg-type]


def handle_webhook(bot: Bot, body: bytes, signature: Optional[str] = None, secret: Optional[str] = None) -> Tuple[int, bytes]:
    """Pure function: (status, response body) for a POSTed envelope. Framework-agnostic (Flask/FastAPI/...)."""
    if secret is not None and not verify(secret, body, signature):
        return 401, b'{"error":{"code":"unauthorized","message":"bad signature"}}'
    env = json.loads(body)
    if env.get("type") != "your_turn":
        return 200, b"{}"
    state = YourTurn.from_dict(env.get("data") or {}, table_id=env.get("table_id", ""), t_ms=int(env.get("t_ms") or 0),
                               hand_id=env.get("hand_id") or "", seq=int(env.get("seq") or 0))
    try:
        a = bot.act(state)
        if not isinstance(a, Action):
            a = safe_action(state)
        elif not bot.strict:
            a = legalize(state, a)
    except Exception:
        a = safe_action(state)
    return 200, json.dumps(a.to_data(state.hand_id, state.decision_id)).encode()


def serve_webhook(bot: Bot, host: str = "0.0.0.0", port: int = 8000, secret: Optional[str] = None,
                  path: str = "/act", block: bool = True) -> ThreadingHTTPServer:
    """Serve `POST {path}` (your_turn -> act) and `POST /events` (fire-and-forget events)."""

    class H(BaseHTTPRequestHandler):
        def do_POST(self) -> None:  # noqa: N802
            n = int(self.headers.get("Content-Length") or 0)
            body = self.rfile.read(n)
            if self.path.rstrip("/") == path.rstrip("/"):
                # the server sends X-Sharingan-Signature; the older X-Signature is still accepted
                sig = self.headers.get("X-Sharingan-Signature") or self.headers.get("X-Signature")
                status, out = handle_webhook(bot, body, sig, secret)
            else:
                status, out = 200, b"{}"
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(out)))
            self.end_headers()
            self.wfile.write(out)

        def log_message(self, *a: Any) -> None:
            pass

    srv = ThreadingHTTPServer((host, port), H)
    if block:
        try:
            srv.serve_forever()
        except KeyboardInterrupt:
            pass
    else:
        threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv
