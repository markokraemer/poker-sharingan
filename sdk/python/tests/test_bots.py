import json
import random

from conftest import DOC_YOUR_TURN
from sharingan import Action, YourTurn
from sharingan.bots import CallingStation, LLMBot, RandomBot, TightAggressive
from sharingan.helpers import is_legal

CARDS = [r + s for r in "23456789TJQKA" for s in "cdhs"]


def random_state(rng: random.Random) -> YourTurn:
    d = json.loads(json.dumps(DOC_YOUR_TURN["data"]))
    street = rng.choice(["preflop", "flop", "turn", "river"])
    cards = rng.sample(CARDS, 7)
    d["hole_cards"], d["street"] = cards[:2], street
    d["board"] = cards[2:2 + {"preflop": 0, "flop": 3, "turn": 4, "river": 5}[street]]
    stack = rng.choice([300, 2000, 9750])
    d["players"][0]["stack"] = stack
    facing = rng.random() < 0.6
    if facing:
        amt = rng.choice([100, 250, 600, stack])
        amt = min(amt, stack)
        d["legal"] = {"fold": True, "check": False, "call": {"to": amt, "added": amt, "all_in": amt == stack},
                      "raise": None if amt >= stack or rng.random() < 0.2 else {"min_to": min(2 * amt, stack), "max_to": stack}}
    else:
        d["legal"] = {"fold": True, "check": True, "call": None,
                      "raise": {"min_to": min(100, stack), "max_to": stack}}
    return YourTurn.from_dict(d)


def test_bots_always_legal():
    rng = random.Random(1)
    bots = [RandomBot(seed=3), CallingStation(), TightAggressive(),
            LLMBot(lambda p: rng.choice(["ACTION: raise 999999", "ACTION: fold", "banana", "ACTION: call"]))]
    for _ in range(400):
        s = random_state(rng)
        for b in bots:
            a = b.act(s)
            assert isinstance(a, Action)
            if not isinstance(b, LLMBot):  # LLM output is untrusted; the runner legalizes it
                assert is_legal(s, a), (b.name, a, s.legal)


def test_calling_station_and_tag_basics():
    rng = random.Random(2)
    s = random_state(rng)
    s.legal.call = None
    s.legal.check = True
    assert CallingStation().act(s) == Action.check()
    s.street, s.board, s.hole_cards = "preflop", [], ["Ah", "Ad"]
    a = TightAggressive().act(s)
    assert a.type == "raise"
    s.hole_cards = ["7h", "2d"]
    assert TightAggressive().act(s).type in ("check", "fold")


def test_llm_bot_prompt_and_parse():
    seen = []

    def complete(prompt: str) -> str:
        seen.append(prompt)
        return "The pot is small.\nACTION: raise 300"

    bot = LLMBot(complete)
    rng = random.Random(5)
    s = random_state(rng)
    assert bot.act(s) == Action("raise", 300)
    assert "Your hole cards" in seen[0] and "Legal actions" in seen[0]
