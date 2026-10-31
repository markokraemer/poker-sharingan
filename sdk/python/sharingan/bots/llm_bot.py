"""An LLM plays: the state is rendered as text, YOU supply the completion callable (no keys/provider in the SDK).

    def complete(prompt: str) -> str:          # any provider; sync or async
        return my_client.chat(prompt)

    run(LLMBot(complete), url, token)
"""
from __future__ import annotations

import inspect
from typing import Awaitable, Callable, Optional, Union

from ..bot import Bot
from ..helpers import parse_action_text, render_state, safe_action
from ..models import Action, YourTurn

SYSTEM_PROMPT = (
    "You are an expert no-limit hold'em player. Think briefly, then end your reply with exactly one final line "
    "of the form `ACTION: fold`, `ACTION: check`, `ACTION: call`, `ACTION: raise <to>` or `ACTION: all-in`. "
    "<to> is the TOTAL chips you will have in the pot this street after raising (not the increment) and must be "
    "within the legal range shown."
)


class LLMBot(Bot):
    """No class-level name: the CLI names the bot after its file (so two LLM bots are two leaderboard rows). Pass `name=` to choose."""

    def __init__(self, complete: Callable[[str], Union[str, Awaitable[str]]], system_prompt: str = SYSTEM_PROMPT,
                 name: Optional[str] = None):
        if name:
            self.name = name
        self.complete = complete
        self.system_prompt = system_prompt
        self.last_prompt = ""
        self.last_reply = ""
        if inspect.iscoroutinefunction(complete):  # async provider client -> async act (runner awaits it)
            self.act = self._act_async  # type: ignore[assignment]

    def build_prompt(self, state: YourTurn) -> str:
        return f"{self.system_prompt}\n\n{render_state(state)}\n\nYour decision?"

    def _parse(self, state: YourTurn, reply: str) -> Action:
        self.last_reply = reply
        a = parse_action_text(reply.split("ACTION:")[-1] if "ACTION:" in reply else reply)
        # the runner legalizes (clamps amounts, impossible raise -> call/check); unparsable -> check/fold
        return a if a is not None else safe_action(state)

    def act(self, state: YourTurn) -> Action:
        self.last_prompt = self.build_prompt(state)
        return self._parse(state, self.complete(self.last_prompt))  # type: ignore[arg-type]

    async def _act_async(self, state: YourTurn) -> Action:
        self.last_prompt = self.build_prompt(state)
        return self._parse(state, await self.complete(self.last_prompt))  # type: ignore[misc]
