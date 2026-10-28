"""Typed models for every payload in docs/TABLE_API.md (stdlib dataclasses only).

All `from_dict` constructors are tolerant: unknown keys are ignored and missing
optional keys fall back to defaults, so a server adding fields never breaks a bot.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional

Card = str  # "Ah", "Td", "2c"
STREETS = ("preflop", "flop", "turn", "river")


@dataclass
class CallOption:
    to: int
    added: int
    all_in: bool = False

    @classmethod
    def from_dict(cls, d: Optional[dict]) -> Optional["CallOption"]:
        if not d:
            return None
        return cls(to=int(d["to"]), added=int(d.get("added", 0)), all_in=bool(d.get("all_in", False)))


@dataclass
class RaiseOption:
    min_to: int
    max_to: int

    @classmethod
    def from_dict(cls, d: Optional[dict]) -> Optional["RaiseOption"]:
        if not d:
            return None
        return cls(min_to=int(d["min_to"]), max_to=int(d["max_to"]))


@dataclass
class Legal:
    fold: bool = True
    check: bool = False
    call: Optional[CallOption] = None
    raise_: Optional[RaiseOption] = None  # `raise` is a keyword; JSON key is "raise"

    @classmethod
    def from_dict(cls, d: dict) -> "Legal":
        return cls(
            fold=bool(d.get("fold", True)),
            check=bool(d.get("check", False)),
            call=CallOption.from_dict(d.get("call")),
            raise_=RaiseOption.from_dict(d.get("raise")),
        )


@dataclass
class ActionEvent:
    hand_id: str = ""
    seq: int = 0
    action_id: str = ""
    street: str = "preflop"
    seat: int = 0
    type: str = ""
    to: int = 0
    added: int = 0
    all_in: bool = False
    forced: bool = False
    timeout: bool = False
    stack: int = 0
    pot: int = 0
    current_bet: int = 0
    min_raise_to: int = 0
    next_to_act: Optional[int] = None

    @classmethod
    def from_dict(cls, d: dict) -> "ActionEvent":
        return cls(
            hand_id=d.get("hand_id", ""), seq=int(d.get("seq", 0)), action_id=d.get("action_id", ""),
            street=d.get("street", "preflop"), seat=int(d.get("seat", 0)), type=d.get("type", ""),
            to=int(d.get("to", 0)), added=int(d.get("added", 0)), all_in=bool(d.get("all_in", False)),
            forced=bool(d.get("forced", False)), timeout=bool(d.get("timeout", False)),
            stack=int(d.get("stack", 0)), pot=int(d.get("pot", 0)), current_bet=int(d.get("current_bet", 0)),
            min_raise_to=int(d.get("min_raise_to", 0)), next_to_act=d.get("next_to_act"),
        )


@dataclass
class PlayerState:
    seat: int
    name: str = ""
    stack_start: int = 0
    stack: int = 0
    street_contrib: int = 0
    hand_contrib: int = 0
    status: str = "active"  # active | folded | all_in | out

    @classmethod
    def from_dict(cls, d: dict) -> "PlayerState":
        return cls(seat=int(d["seat"]), name=d.get("name", ""), stack_start=int(d.get("stack_start", 0)),
                   stack=int(d.get("stack", 0)), street_contrib=int(d.get("street_contrib", 0)),
                   hand_contrib=int(d.get("hand_contrib", 0)), status=d.get("status", "active"))


@dataclass
class Pot:
    amount: int
    eligible_seats: List[int] = field(default_factory=list)
    winners: List[int] = field(default_factory=list)  # hand_end only
    share: List[int] = field(default_factory=list)    # hand_end only

    @classmethod
    def from_dict(cls, d: dict) -> "Pot":
        return cls(amount=int(d["amount"]), eligible_seats=list(d.get("eligible_seats", [])),
                   winners=list(d.get("winners", [])), share=list(d.get("share", [])))


@dataclass
class YourTurn:
    """The decision state handed to `Bot.act`. Convenience helpers live in `sharingan.helpers`
    and are also exposed as properties/methods here."""
    hand_id: str = ""
    seq: int = 0
    decision_id: str = ""
    seat: int = 0
    street: str = "preflop"
    hole_cards: List[Card] = field(default_factory=list)
    board: List[Card] = field(default_factory=list)
    history: List[ActionEvent] = field(default_factory=list)
    players: List[PlayerState] = field(default_factory=list)
    button_seat: int = 0
    small_blind: int = 0
    big_blind: int = 0
    pot: int = 0
    pots: List[Pot] = field(default_factory=list)
    legal: Legal = field(default_factory=Legal)
    deadline_ms: int = 0
    time_bank_ms: int = 0
    table_id: str = ""  # from the envelope
    t_ms: int = 0       # from the envelope

    @classmethod
    def from_dict(cls, d: dict, table_id: str = "", t_ms: int = 0, hand_id: str = "", seq: int = 0) -> "YourTurn":
        """`hand_id`/`seq` fall back to the envelope values (the doc example only carries them on the envelope)."""
        return cls(
            hand_id=d.get("hand_id") or hand_id, seq=int(d.get("seq") or seq or 0), decision_id=d.get("decision_id", ""),
            seat=int(d.get("seat", 0)), street=d.get("street", "preflop"),
            hole_cards=list(d.get("hole_cards", [])), board=list(d.get("board", [])),
            history=[ActionEvent.from_dict(a) for a in d.get("history", [])],
            players=[PlayerState.from_dict(p) for p in d.get("players", [])],
            button_seat=int(d.get("button_seat", 0)), small_blind=int(d.get("small_blind", 0)),
            big_blind=int(d.get("big_blind", 0)), pot=int(d.get("pot", 0)),
            pots=[Pot.from_dict(p) for p in d.get("pots", [])],
            legal=Legal.from_dict(d.get("legal", {})), deadline_ms=int(d.get("deadline_ms", 0)),
            time_bank_ms=int(d.get("time_bank_ms", 0)), table_id=d.get("table_id", table_id), t_ms=t_ms,
        )

    # --- convenience (thin wrappers; logic in helpers.py) ---
    @property
    def me(self) -> Optional[PlayerState]:
        for p in self.players:
            if p.seat == self.seat:
                return p
        return None

    @property
    def to_call(self) -> int:
        """Chips needed to call (0 if checking is free)."""
        return self.legal.call.added if self.legal.call else 0

    @property
    def can_check(self) -> bool:
        return self.legal.check

    @property
    def can_raise(self) -> bool:
        return self.legal.raise_ is not None

    @property
    def min_raise_to(self) -> Optional[int]:
        return self.legal.raise_.min_to if self.legal.raise_ else None

    @property
    def max_raise_to(self) -> Optional[int]:
        return self.legal.raise_.max_to if self.legal.raise_ else None

    @property
    def pot_odds(self) -> float:
        """Fraction of the final pot you must put in to call: to_call / (pot + to_call)."""
        tc = self.to_call
        return tc / (self.pot + tc) if tc > 0 else 0.0

    @property
    def stack(self) -> int:
        m = self.me
        return m.stack if m else 0

    @property
    def num_active(self) -> int:
        return sum(1 for p in self.players if p.status in ("active", "all_in"))

    def seconds_left(self, now_ms: Optional[int] = None) -> float:
        import time
        now = now_ms if now_ms is not None else int(time.time() * 1000)
        return (self.deadline_ms - now) / 1000.0


@dataclass
class HandStartPlayer:
    seat: int
    player_id: str = ""
    name: str = ""
    stack_start: int = 0
    in_hand: bool = True


@dataclass
class HandStart:
    hand_id: str = ""
    hand_no: int = 0
    match_id: Optional[str] = None
    deal_no: Optional[int] = None
    rotation: Optional[int] = None
    button_seat: int = 0
    sb_seat: int = 0
    bb_seat: int = 0
    small_blind: int = 0
    big_blind: int = 0
    ante: int = 0
    players: List[HandStartPlayer] = field(default_factory=list)
    deck_commitment: str = ""

    @classmethod
    def from_dict(cls, d: dict) -> "HandStart":
        return cls(
            hand_id=d.get("hand_id", ""), hand_no=int(d.get("hand_no", 0)), match_id=d.get("match_id"),
            deal_no=d.get("deal_no"), rotation=d.get("rotation"), button_seat=int(d.get("button_seat", 0)),
            sb_seat=int(d.get("sb_seat", 0)), bb_seat=int(d.get("bb_seat", 0)),
            small_blind=int(d.get("small_blind", 0)), big_blind=int(d.get("big_blind", 0)),
            ante=int(d.get("ante", 0)),
            players=[HandStartPlayer(seat=int(p["seat"]), player_id=p.get("player_id", ""), name=p.get("name", ""),
                                     stack_start=int(p.get("stack_start", 0)), in_hand=bool(p.get("in_hand", True)))
                     for p in d.get("players", [])],
            deck_commitment=d.get("deck_commitment", ""),
        )


@dataclass
class StreetEvent:
    hand_id: str = ""
    street: str = ""
    board: List[Card] = field(default_factory=list)
    new_cards: List[Card] = field(default_factory=list)
    pot: int = 0
    pots: List[Pot] = field(default_factory=list)

    @classmethod
    def from_dict(cls, d: dict) -> "StreetEvent":
        return cls(hand_id=d.get("hand_id", ""), street=d.get("street", ""), board=list(d.get("board", [])),
                   new_cards=list(d.get("new_cards", [])), pot=int(d.get("pot", 0)),
                   pots=[Pot.from_dict(p) for p in d.get("pots", [])])


@dataclass
class ShowdownEntry:
    seat: int
    cards: List[Card] = field(default_factory=list)
    shown: bool = False
    hand_rank: str = ""
    best5: List[Card] = field(default_factory=list)


@dataclass
class SeatResult:
    seat: int
    player_id: str = ""
    net: int = 0
    stack_end: int = 0


@dataclass
class HandEnd:
    hand_id: str = ""
    seq: int = 0
    board: List[Card] = field(default_factory=list)
    showdown: List[ShowdownEntry] = field(default_factory=list)
    hole_cards: Dict[int, List[Card]] = field(default_factory=dict)  # every dealt player
    pots: List[Pot] = field(default_factory=list)
    results: List[SeatResult] = field(default_factory=list)
    deck_reveal: Dict[str, Any] = field(default_factory=dict)
    ended_by: str = "showdown"

    @classmethod
    def from_dict(cls, d: dict) -> "HandEnd":
        return cls(
            hand_id=d.get("hand_id", ""), seq=int(d.get("seq", 0)), board=list(d.get("board", [])),
            showdown=[ShowdownEntry(seat=int(s["seat"]), cards=list(s.get("cards", [])), shown=bool(s.get("shown", False)),
                                    hand_rank=s.get("hand_rank", ""), best5=list(s.get("best5", [])))
                      for s in d.get("showdown", [])],
            # JSON object keys are strings -> normalise to int seats
            hole_cards={int(k): list(v) for k, v in (d.get("hole_cards") or {}).items()},
            pots=[Pot.from_dict(p) for p in d.get("pots", [])],
            results=[SeatResult(seat=int(r["seat"]), player_id=r.get("player_id", ""), net=int(r.get("net", 0)),
                                stack_end=int(r.get("stack_end", 0))) for r in d.get("results", [])],
            deck_reveal=d.get("deck_reveal") or {}, ended_by=d.get("ended_by", "showdown"),
        )

    def net_for(self, seat: int) -> int:
        for r in self.results:
            if r.seat == seat:
                return r.net
        return 0


@dataclass
class ApiError:
    code: str
    message: str = ""
    decision_id: Optional[str] = None
    details: Dict[str, Any] = field(default_factory=dict)

    @classmethod
    def from_dict(cls, d: dict) -> "ApiError":
        return cls(code=d.get("code", "internal"), message=d.get("message", ""),
                   decision_id=d.get("decision_id"), details=d.get("details") or {})


@dataclass
class Seat:
    seat: int
    player_id: Optional[str] = None
    name: Optional[str] = None
    stack: int = 0
    status: str = "empty"

    @classmethod
    def from_dict(cls, d: dict) -> "Seat":
        return cls(seat=int(d["seat"]), player_id=d.get("player_id"), name=d.get("name"),
                   stack=int(d.get("stack", 0)), status=d.get("status", "empty"))


@dataclass
class Table:
    table_id: str
    config: Dict[str, Any] = field(default_factory=dict)
    status: str = "waiting"
    seats: List[Seat] = field(default_factory=list)
    button_seat: Optional[int] = None
    hand_id: Optional[str] = None
    table_seq: int = 0
    hand: Optional[Dict[str, Any]] = None  # raw HandState from table_snapshot / GET table
    raw: Dict[str, Any] = field(default_factory=dict)

    @classmethod
    def from_dict(cls, d: dict) -> "Table":
        return cls(table_id=d.get("table_id", ""), config=d.get("config") or {}, status=d.get("status", "waiting"),
                   seats=[Seat.from_dict(s) for s in d.get("seats", [])], button_seat=d.get("button_seat"),
                   hand_id=d.get("hand_id"), table_seq=int(d.get("table_seq", 0)), hand=d.get("hand"), raw=d)


@dataclass
class PlayerInfo:
    player_id: str
    name: str = ""
    kind: str = "bot"
    token: Optional[str] = None  # only returned on creation
    raw: Dict[str, Any] = field(default_factory=dict)

    @classmethod
    def from_dict(cls, d: dict) -> "PlayerInfo":
        return cls(player_id=d.get("player_id", ""), name=d.get("name", ""), kind=d.get("kind", "bot"),
                   token=d.get("token"), raw=d)


@dataclass
class Action:
    """What a bot returns. `to` is the TOTAL street contribution after the action ("to", never "by")."""
    type: str
    to: Optional[int] = None

    @staticmethod
    def fold() -> "Action":
        return Action("fold")

    @staticmethod
    def check() -> "Action":
        return Action("check")

    @staticmethod
    def call() -> "Action":
        return Action("call")

    @staticmethod
    def raise_to(to: int) -> "Action":
        return Action("raise", int(to))

    @staticmethod
    def bet(to: int) -> "Action":
        return Action("bet", int(to))

    def to_data(self, hand_id: str, decision_id: str) -> Dict[str, Any]:
        d: Dict[str, Any] = {"hand_id": hand_id, "decision_id": decision_id, "type": self.type}
        if self.to is not None and self.type in ("raise", "bet", "call"):
            d["to"] = int(self.to)
        return d
