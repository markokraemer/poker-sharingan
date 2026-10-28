"""Pure helper functions: legal actions, pot odds, raise sizing ("to" amounts), hand strength, text rendering."""
from __future__ import annotations

from typing import List, Optional, Tuple

from .models import Action, YourTurn

RANKS = "23456789TJQKA"


def rank_value(card: str) -> int:
    """2..14"""
    return RANKS.index(card[0]) + 2


def legal_types(s: YourTurn) -> List[str]:
    """Action types the bot may send right now: subset of fold/check/call/raise."""
    out = []
    if s.legal.fold:
        out.append("fold")
    if s.legal.check:
        out.append("check")
    if s.legal.call:
        out.append("call")
    if s.legal.raise_:
        out.append("raise")
    return out


def to_call(s: YourTurn) -> int:
    return s.to_call


def pot_odds(s: YourTurn) -> float:
    """to_call / (pot + to_call). Equity needed to break even on a call."""
    return s.pot_odds


def min_raise_to(s: YourTurn) -> Optional[int]:
    return s.min_raise_to


def max_raise_to(s: YourTurn) -> Optional[int]:
    return s.max_raise_to


def my_street_contrib(s: YourTurn) -> int:
    m = s.me
    return m.street_contrib if m else 0


def current_bet(s: YourTurn) -> int:
    """Highest street contribution on the table (what a call would bring us to)."""
    if s.legal.call:
        return s.legal.call.to
    return my_street_contrib(s)  # checking is free -> we are level with the bet


def pot_raise_to(s: YourTurn, fraction: float = 1.0) -> int:
    """'to' amount for a raise/bet of `fraction` x pot (after calling), clamped to the legal range.

    Standard pot-sized raise: call first, then raise by (pot + to_call) * fraction.
    """
    base = current_bet(s)
    to = base + int(round((s.pot + s.to_call) * fraction))
    return clamp_raise_to(s, to)


def clamp_raise_to(s: YourTurn, to: int) -> int:
    r = s.legal.raise_
    if r is None:
        raise ValueError("raising is not legal")
    return max(r.min_to, min(r.max_to, int(to)))


def bb_multiple_to(s: YourTurn, bbs: float) -> int:
    """'to' amount of `bbs` big blinds, clamped to the legal range."""
    return clamp_raise_to(s, int(round(bbs * s.big_blind)))


def all_in_action(s: YourTurn) -> Action:
    if s.legal.raise_:
        return Action.raise_to(s.legal.raise_.max_to)
    if s.legal.call:
        return Action.call()
    return safe_action(s)


def safe_action(s: YourTurn) -> Action:
    """Check if free, else fold. Same as the server's timeout policy."""
    return Action.check() if s.legal.check else Action.fold()


def call_or_check(s: YourTurn) -> Action:
    if s.legal.call:
        return Action.call()
    if s.legal.check:
        return Action.check()
    return Action.fold()


def legalize(s: YourTurn, a: Action) -> Action:
    """Coerce any action into one that is legal in this spot (never raises, never returns an illegal action).

    - fold when check is free -> check
    - raise when raising is impossible -> call/check
    - raise amounts clamped to [min_to, max_to]; a `bet` is sent as `raise` semantics-equivalent type
    - check when facing a bet -> fold
    - call with nothing to call -> check
    """
    t = a.type
    if t in ("bet", "raise", "all_in"):
        if s.legal.raise_:
            to = a.to if (a.to is not None and t != "all_in") else s.legal.raise_.max_to
            return Action("raise", clamp_raise_to(s, to))
        return call_or_check(s)
    if t == "call":
        return call_or_check(s)
    if t == "check":
        return Action.check() if s.legal.check else (Action.fold())
    if t == "fold":
        return Action.check() if s.legal.check else Action.fold()
    return safe_action(s)


def is_legal(s: YourTurn, a: Action) -> bool:
    """True if `a` can be sent as-is without the server rejecting it."""
    if a.type == "fold":
        return s.legal.fold
    if a.type == "check":
        return s.legal.check
    if a.type == "call":
        return s.legal.call is not None
    if a.type in ("raise", "bet"):
        r = s.legal.raise_
        return r is not None and a.to is not None and r.min_to <= a.to <= r.max_to
    return False


def effective_stack_bb(s: YourTurn) -> float:
    """Our stack in big blinds."""
    return s.stack / s.big_blind if s.big_blind else 0.0


def position_name(s: YourTurn) -> str:
    """BTN, SB, BB, UTG, MP, CO (6-max naming, relative to occupied seats)."""
    seats = sorted(p.seat for p in s.players)
    if s.seat not in seats or s.button_seat not in seats and not seats:
        return "?"
    n = len(seats)
    # distance clockwise from button among occupied seats
    order = sorted(seats, key=lambda x: (x - s.button_seat) % 6)
    idx = order.index(s.seat)  # 0 = BTN
    if n == 2:
        return "BTN/SB" if idx == 0 else "BB"
    names = {0: "BTN", 1: "SB", 2: "BB"}
    if idx in names:
        return names[idx]
    rest = n - 3  # UTG.. CO
    pos = idx - 3
    if pos == rest - 1:
        return "CO"
    if pos == 0:
        return "UTG"
    return "MP"


def preflop_strength(hole: List[str]) -> float:
    """Chen formula score (roughly -1..20). >=9 premium, 7+ strong, 5+ playable."""
    if len(hole) != 2:
        return 0.0
    r = sorted((rank_value(hole[0]), rank_value(hole[1])), reverse=True)
    hi, lo = r
    pts = {14: 10, 13: 8, 12: 7, 11: 6}.get(hi, hi / 2.0)
    if hi == lo:
        return max(5.0, pts * 2)
    if hole[0][1] == hole[1][1]:
        pts += 2
    gap = hi - lo - 1
    pts -= {0: 0, 1: 1, 2: 2, 3: 4}.get(gap, 5)
    if gap <= 1 and hi < 12:
        pts += 1
    return float(round(pts * 2) / 2)


def made_hand_category(hole: List[str], board: List[str]) -> int:
    """Very coarse postflop strength: 0 air, 1 pair (any), 2 two pair, 3 trips+ (trips, straight, flush, FH, quads).

    Good enough for rule-based example bots. Not a hand evaluator.
    """
    cards = list(hole) + list(board)
    if len(cards) < 5:
        return 0
    from collections import Counter
    rc = Counter(c[0] for c in cards)
    sc = Counter(c[1] for c in cards)
    counts = sorted(rc.values(), reverse=True)
    vals = sorted({rank_value(c) for c in cards})
    if 14 in vals:
        vals = [1] + vals
    straight = any(all(v + i in vals for i in range(5)) for v in range(1, 11))
    flush = max(sc.values()) >= 5
    if counts[0] >= 3 or straight or flush:
        return 3
    if counts[0] == 2 and counts[1] == 2:
        return 2
    if counts[0] == 2:
        # only count the pair if it involves a hole card (board pair alone is not "ours")
        paired = [r for r, c in rc.items() if c == 2]
        return 1 if any(r in (hole[0][0], hole[1][0]) for r in paired) else 0
    return 0


def render_state(s: YourTurn) -> str:
    """Plain-text rendering of the decision for LLM prompts or logs."""
    lines: List[str] = []
    lines.append(f"Texas Hold'em no-limit, 6-max. Blinds {s.small_blind}/{s.big_blind}. Street: {s.street}.")
    lines.append(f"You are seat {s.seat} ({position_name(s)}), stack {s.stack} ({effective_stack_bb(s):.1f} bb).")
    lines.append(f"Your hole cards: {' '.join(s.hole_cards)}")
    lines.append(f"Board: {' '.join(s.board) if s.board else '(none)'}")
    lines.append(f"Pot: {s.pot}. To call: {s.to_call}. Pot odds: {s.pot_odds:.1%}.")
    lines.append("Players:")
    for p in s.players:
        tag = " (YOU)" if p.seat == s.seat else ""
        lines.append(f"  seat {p.seat} {p.name}{tag}: stack {p.stack}, in pot this street {p.street_contrib}, "
                     f"this hand {p.hand_contrib}, {p.status}")
    lines.append("Action so far:")
    for a in s.history:
        if a.type in ("fold", "check"):
            desc = a.type
        else:
            desc = f"{a.type} to {a.to}" + (" (all-in)" if a.all_in else "")
        lines.append(f"  [{a.street}] seat {a.seat}: {desc}")
    opts = []
    if s.legal.fold:
        opts.append("fold")
    if s.legal.check:
        opts.append("check")
    if s.legal.call:
        opts.append(f"call (adds {s.legal.call.added}, to {s.legal.call.to}{', all-in' if s.legal.call.all_in else ''})")
    if s.legal.raise_:
        opts.append(f"raise (to an amount between {s.legal.raise_.min_to} and {s.legal.raise_.max_to}; "
                    f"amounts are TOTAL street contribution, not increments)")
    lines.append("Legal actions: " + "; ".join(opts))
    return "\n".join(lines)


def parse_action_text(text: str) -> Optional[Action]:
    """Parse free-form LLM output like 'raise 450', 'RAISE TO 450', 'call', 'fold', 'check', 'all-in'.

    Returns None if nothing recognisable. Uses the LAST recognised action word in the text.
    """
    import re
    t = text.strip().lower()
    found: Optional[Tuple[int, Action]] = None
    for m in re.finditer(r"\b(fold|check|call|all[- ]?in|raise|bet)\b(?:\s*(?:to|=|:)?\s*\$?(\d+))?", t):
        w = m.group(1)
        if w.startswith("all"):
            a = Action("all_in")
        elif w in ("raise", "bet"):
            a = Action(w, int(m.group(2))) if m.group(2) else Action("raise", None)
        else:
            a = Action(w)
        found = (m.start(), a)
    return found[1] if found else None
