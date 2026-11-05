"""Operator E2E baseline: the quickstart strategy, without network startup code."""
from sharingan import Action


def act(s):
    if s.legal.raise_ and s.hole_cards[0][0] in "AK":
        return Action.raise_to(3 * s.big_blind)
    if s.legal.call and s.pot_odds < 0.25:
        return Action.call()
    return Action.check() if s.legal.check else Action.fold()
