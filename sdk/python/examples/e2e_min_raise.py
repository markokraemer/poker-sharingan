"""Operator sizing probe: repeatedly raise by the legal minimum."""
from sharingan import Action


def act(s):
    if s.legal.raise_:
        return Action.raise_to(s.legal.raise_.min_to)
    return Action.check() if s.legal.check else Action.call()
