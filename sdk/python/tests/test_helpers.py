import pytest

from conftest import doc_state, facing_bet_state
from sharingan import (Action, legal_types, legalize, parse_action_text, pot_raise_to, preflop_strength,
                       render_state, safe_action)
from sharingan.helpers import (all_in_action, bb_multiple_to, is_legal, made_hand_category, position_name)


def test_parse_doc_example():
    s = doc_state()
    assert s.decision_id == "d-77-9" and s.table_id == "t1"
    assert s.hole_cards == ["Ah", "Kd"] and s.board == ["7c", "8d", "2s"]
    assert s.legal.check and s.legal.call is None and s.legal.raise_.max_to == 9750
    assert s.to_call == 0 and s.pot_odds == 0.0 and s.stack == 9750
    assert s.history[0].type == "post_sb" and s.history[-1].next_to_act == 0
    assert legal_types(s) == ["fold", "check", "raise"]
    assert s.pots[0].eligible_seats == [0, 5]


def test_pot_odds_and_sizes():
    s = facing_bet_state()
    assert s.to_call == 200
    assert s.pot_odds == pytest.approx(200 / 950)
    # call to 200, then raise by pot+call: 200 + (750 + 200) = 1150
    assert pot_raise_to(s, 1.0) == 1150
    assert pot_raise_to(s, 0.0) == 400  # clamped up to min_to
    assert bb_multiple_to(s, 1000) == 9750  # clamped down to max
    assert all_in_action(s) == Action.raise_to(9750)


def test_pot_bet_when_checking():
    s = doc_state()  # pot 550, no bet
    assert pot_raise_to(s, 1.0) == 550  # bet pot
    assert pot_raise_to(s, 0.5) == 275


def test_legalize():
    s = doc_state()
    assert legalize(s, Action.fold()) == Action.check()            # fold when check free -> check
    assert legalize(s, Action.call()) == Action.check()
    assert legalize(s, Action.raise_to(1)) == Action.raise_to(100)  # clamp up
    assert legalize(s, Action.raise_to(10**9)) == Action.raise_to(9750)
    assert legalize(s, Action("all_in")) == Action.raise_to(9750)
    assert legalize(s, Action("bet", 300)) == Action.raise_to(300)
    f = facing_bet_state()
    assert legalize(f, Action.check()) == Action.fold()
    f.legal.raise_ = None
    assert legalize(f, Action.raise_to(500)) == Action.call()
    assert is_legal(s, Action.raise_to(100)) and not is_legal(s, Action.raise_to(99)) and not is_legal(s, Action.call())
    assert safe_action(f) == Action.fold()


def test_action_wire_format():
    d = Action.raise_to(400).to_data("h-77", "d-77-9")
    assert d == {"hand_id": "h-77", "decision_id": "d-77-9", "type": "raise", "to": 400}
    assert Action.fold().to_data("h", "d") == {"hand_id": "h", "decision_id": "d", "type": "fold"}


def test_parse_action_text():
    assert parse_action_text("I think we should RAISE TO 450.") == Action("raise", 450)
    assert parse_action_text("ACTION: call") == Action.call()
    assert parse_action_text("fold") == Action.fold()
    assert parse_action_text("all-in!") == Action("all_in")
    assert parse_action_text("no idea") is None
    assert parse_action_text("call... actually raise 300") == Action("raise", 300)


def test_strength_and_position():
    assert preflop_strength(["Ah", "Ad"]) >= 20 - 1 and preflop_strength(["As", "Ks"]) >= 10
    assert preflop_strength(["7h", "2d"]) < 3
    assert made_hand_category(["Ah", "Ad"], ["7c", "8d", "2s"]) == 1
    assert made_hand_category(["Ah", "Kd"], ["7c", "8d", "2s"]) == 0
    assert made_hand_category(["Ah", "Kd"], ["Ac", "Kc", "2s"]) == 2
    assert made_hand_category(["Ah", "Kd"], ["Qc", "Jc", "Ts"]) == 3  # straight
    s = doc_state()
    s.players = [p for p in s.players]
    assert position_name(s) in ("UTG", "SB", "BB", "BTN", "MP", "CO", "BTN/SB")


def test_render_state():
    txt = render_state(facing_bet_state())
    for needle in ("Ah Kd", "7c 8d 2s", "To call: 200", "raise (to an amount between 400 and 9750", "post_sb to 50"):
        assert needle in txt
