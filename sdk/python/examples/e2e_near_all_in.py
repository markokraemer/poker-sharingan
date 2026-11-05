"""Operator attack probe: leave one chip preflop, then shove postflop."""
from sharingan import Action


def act(s):
    raise_ = s.legal.raise_
    if s.street == "preflop":
        if raise_ and raise_.max_to - 1 >= raise_.min_to:
            return Action.raise_to(raise_.max_to - 1)
        if s.legal.check:
            return Action.check()
        return Action.call() if s.to_call <= 1 else Action.fold()
    if raise_:
        return Action.raise_to(raise_.max_to)
    return Action.check() if s.legal.check else Action.call()
