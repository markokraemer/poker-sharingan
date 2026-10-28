from ..bot import Bot
from ..helpers import (all_in_action, bb_multiple_to, call_or_check, made_hand_category, pot_raise_to,
                       preflop_strength, safe_action)
from ..models import Action, YourTurn


class TightAggressive(Bot):
    """Simple rules: play few hands preflop, raise strong ones, bet made hands postflop, fold to pressure with air."""
    name = "tight_aggressive"

    def act(self, s: YourTurn) -> Action:
        return self.preflop(s) if s.street == "preflop" else self.postflop(s)

    def preflop(self, s: YourTurn) -> Action:
        score = preflop_strength(s.hole_cards)
        facing_raise = s.to_call > s.big_blind
        if score >= 10:  # AA-JJ, AKs...: raise / re-raise / jam when deep into the pot
            if not s.can_raise:
                return call_or_check(s)
            if s.to_call > 0.3 * s.stack:
                return all_in_action(s)
            return Action.raise_to(bb_multiple_to(s, 9 if facing_raise else 3))
        if score >= 7:  # strong: open-raise, call a single raise
            if not facing_raise and s.can_raise:
                return Action.raise_to(bb_multiple_to(s, 3))
            return call_or_check(s) if s.to_call <= 0.1 * s.stack else safe_action(s)
        if score >= 5 and not facing_raise:  # playable: limp-call a min price, never raise
            return call_or_check(s)
        return safe_action(s)

    def postflop(self, s: YourTurn) -> Action:
        strength = made_hand_category(s.hole_cards, s.board)
        if strength >= 2 and s.can_raise:
            return Action.raise_to(pot_raise_to(s, 0.75))
        if strength == 1:
            if s.to_call == 0 and s.can_raise:
                return Action.raise_to(pot_raise_to(s, 0.5))
            if s.pot_odds <= 0.33:
                return call_or_check(s)
            return safe_action(s)
        if s.to_call == 0:
            return Action.check()
        return Action.fold()
