from ..bot import Bot
from ..helpers import call_or_check
from ..models import Action, YourTurn


class CallingStation(Bot):
    """Never folds to a bet, never raises: checks when free, otherwise calls."""
    name = "calling_station"

    def act(self, state: YourTurn) -> Action:
        return call_or_check(state)
