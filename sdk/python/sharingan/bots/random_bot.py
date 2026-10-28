import random
from typing import Optional

from ..bot import Bot
from ..helpers import bb_multiple_to, legal_types, safe_action
from ..models import Action, YourTurn


class RandomBot(Bot):
    """Picks uniformly among legal action types; raises a random 2-10 bb (clamped)."""
    name = "random"

    def __init__(self, seed: Optional[int] = None):
        self.rng = random.Random(seed)

    def act(self, state: YourTurn) -> Action:
        t = self.rng.choice(legal_types(state))
        if t == "raise":
            return Action.raise_to(bb_multiple_to(state, self.rng.uniform(2, 10)))
        return {"fold": Action.fold, "check": Action.check, "call": Action.call}[t]()
