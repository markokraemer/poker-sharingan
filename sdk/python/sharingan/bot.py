"""Bot base class: implement `act(state) -> Action`; override the on_* hooks if you want to track hands."""
from __future__ import annotations

from .models import Action, ApiError, HandEnd, HandStart, StreetEvent, ActionEvent, YourTurn


class Bot:
    """Subclass and implement `act`. It may be a plain method or `async def`.

    Whatever you return is passed through `helpers.legalize` by the runner (unless
    `strict = True`), so an off-by-a-chip raise becomes a legal one instead of an error.
    If `act` raises or is too slow the runner plays check/fold (the server's own timeout policy).
    """

    name: str = "bot"
    strict: bool = False  # True = send exactly what act() returned (the server may reject it)

    def act(self, state: YourTurn) -> Action:  # pragma: no cover - interface
        raise NotImplementedError

    # --- optional hooks (all may be sync or async) ---
    def on_connect(self, welcome: dict) -> None: ...
    def on_hand_start(self, hand: HandStart) -> None: ...
    def on_action(self, event: ActionEvent) -> None: ...
    def on_street(self, street: StreetEvent) -> None: ...
    def on_hand_end(self, end: HandEnd) -> None: ...
    def on_error(self, error: ApiError) -> None: ...
