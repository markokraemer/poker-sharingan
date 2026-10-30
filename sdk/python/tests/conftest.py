import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(__file__)))

import pytest


@pytest.fixture(autouse=True)
def _isolated_tokens(tmp_path, monkeypatch):
    # The CLI saves bot tokens in ~/.config/sharingan; keep tests out of the real home.
    import sharingan.cli
    monkeypatch.setattr(sharingan.cli, "TOKENS_FILE", str(tmp_path / "tokens.json"))

from sharingan.models import YourTurn

# The worked example from docs/TABLE_API.md section 5.3 (seat 0 on the flop, facing a check)
DOC_YOUR_TURN = {
    "v": 1, "type": "your_turn", "table_id": "t1", "hand_id": "h-77", "seq": 9, "t_ms": 1792000000000,
    "data": {
        "decision_id": "d-77-9", "seat": 0, "street": "flop", "hole_cards": ["Ah", "Kd"], "board": ["7c", "8d", "2s"],
        "history": [
            {"seq": 1, "street": "preflop", "seat": 4, "type": "post_sb", "to": 50, "added": 50, "all_in": False, "forced": True, "timeout": False, "stack": 9950, "pot": 50, "current_bet": 50, "min_raise_to": 100, "next_to_act": 5},
            {"seq": 2, "street": "preflop", "seat": 5, "type": "post_bb", "to": 100, "added": 100, "all_in": False, "forced": True, "timeout": False, "stack": 9900, "pot": 150, "current_bet": 100, "min_raise_to": 200, "next_to_act": 0},
            {"seq": 3, "street": "preflop", "seat": 0, "type": "raise", "to": 250, "added": 250, "all_in": False, "forced": False, "timeout": False, "stack": 9750, "pot": 400, "current_bet": 250, "min_raise_to": 400, "next_to_act": 1},
            {"seq": 8, "street": "flop", "seat": 5, "type": "check", "to": 0, "added": 0, "all_in": False, "forced": False, "timeout": False, "stack": 9750, "pot": 550, "current_bet": 0, "min_raise_to": 100, "next_to_act": 0},
        ],
        "players": [
            {"seat": 0, "name": "pluribus", "stack_start": 10000, "stack": 9750, "street_contrib": 0, "hand_contrib": 250, "status": "active"},
            {"seat": 5, "name": "erin", "stack_start": 10000, "stack": 9750, "street_contrib": 0, "hand_contrib": 250, "status": "active"},
        ],
        "button_seat": 3, "small_blind": 50, "big_blind": 100, "pot": 550,
        "pots": [{"amount": 550, "eligible_seats": [0, 5]}],
        "legal": {"fold": True, "check": True, "call": None, "raise": {"min_to": 100, "max_to": 9750}},
        "deadline_ms": 1792000005000, "time_bank_ms": 30000,
    },
}


def doc_state() -> YourTurn:
    return YourTurn.from_dict(DOC_YOUR_TURN["data"], table_id="t1", t_ms=DOC_YOUR_TURN["t_ms"])


def facing_bet_state(**over) -> YourTurn:
    d = json.loads(json.dumps(DOC_YOUR_TURN["data"]))
    d["pot"] = 750
    d["players"][0]["street_contrib"] = 0
    d["players"][1]["street_contrib"] = 200
    d["legal"] = {"fold": True, "check": False, "call": {"to": 200, "added": 200, "all_in": False},
                  "raise": {"min_to": 400, "max_to": 9750}}
    d.update(over)
    return YourTurn.from_dict(d, table_id="t1", t_ms=1792000000000)
