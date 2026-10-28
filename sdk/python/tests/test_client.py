import asyncio
import json
import logging

import pytest

from sharingan import (Action, ApiException, ArenaClient, Bot, Runner, handle_webhook, ws_url)
from sharingan.bots import CallingStation, LLMBot, RandomBot, TightAggressive
from sharingan.mock import MockArena
from sharingan.webhook import sign

from conftest import DOC_YOUR_TURN


def arun(coro):
    return asyncio.run(asyncio.wait_for(coro, 20))


async def play(arena, bot, hands, **kw):
    r = Runner(bot, arena.ws_url, "any-token", max_hands=hands, **kw)
    await r.run()
    return r


def test_ws_url():
    assert ws_url("http://h:1", "tk") == "ws://h:1/v1/play?token=tk"
    assert ws_url("https://h", "tk") == "wss://h/v1/play?token=tk"
    assert ws_url("wss://h/v1/play?token=x", "tk") == "wss://h/v1/play?token=x"


def test_rest_flow():
    async def main():
        async with MockArena() as arena:
            def sync():
                api = ArenaClient(arena.http_url)
                me = api.register_bot("my-bot")
                assert me.token and me.player_id and api.token == me.token
                t = api.create_table(small_blind=50)
                assert t.config["big_blind"] == 100 and len(t.seats) == 6
                seat = api.join_table(t.table_id, 2, me.player_id)
                assert seat.player_id == me.player_id and seat.status == "seated"
                with pytest.raises(ApiException) as e:
                    api.join_table(t.table_id, 2, me.player_id)
                assert e.value.code == "seat_taken" and e.value.status == 409
                assert api.get_table(t.table_id).seats[2].name == "my-bot"
                with pytest.raises(ApiException) as e:
                    ArenaClient(arena.http_url, token="nope").get_table(t.table_id)
                assert e.value.code == "unauthorized"
            await asyncio.to_thread(sync)
    arun(main())


@pytest.mark.parametrize("make", [lambda: RandomBot(seed=1), CallingStation, TightAggressive,
                                  lambda: LLMBot(lambda p: "ACTION: call")])
def test_full_session(make):
    async def main():
        async with MockArena(hands=4) as arena:
            r = await play(arena, make(), 4)
            assert r.hands_played == 4 and len(arena.hand_ends) == 4
            assert arena.errors_sent == []            # nothing illegal was ever sent
            assert r.welcome["player_id"] and arena.received[0]["type"] == "hello"
            assert r.net_chips == sum(h["results"][0]["net"] for h in arena.hand_ends)
            for h in arena.hand_ends:
                assert sum(x["net"] for x in h["results"]) == 0
    arun(main())


def test_hooks_and_async_bot():
    events = []

    class B(Bot):
        async def act(self, s):
            await asyncio.sleep(0)
            return Action.call() if s.legal.call else Action.check()
        def on_hand_start(self, h): events.append(("start", h.hand_no, h.bb_seat))
        def on_action(self, a): events.append(("action", a.type))
        def on_street(self, s): events.append(("street", s.street, tuple(s.board)))
        async def on_hand_end(self, e): events.append(("end", e.ended_by, e.hole_cards[0]))

    async def main():
        async with MockArena(hands=1) as arena:
            await play(arena, B(), 1)
    arun(main())
    kinds = [e[0] for e in events]
    assert kinds[0] == "start" and "street" in kinds and kinds[-1] == "end"
    assert ("street", "flop", ("7c", "8d", "2s")) in events
    assert events[-1] == ("end", "showdown", ["Ah", "Kd"])


def test_reconnect_resumes_same_decision():
    async def main():
        async with MockArena(hands=3, drop_on_turns={1, 3}) as arena:
            r = await play(arena, CallingStation(), 3)
            assert r.hands_played == 3
            assert arena.connections >= 3
            hellos = [m for m in arena.received if m["type"] == "hello"]
            assert "resume" not in hellos[0]["data"]
            assert hellos[1]["data"]["resume"][0]["table_id"] == "t1"
            assert hellos[1]["data"]["resume"][0]["last_table_seq"] > 0
            # the re-offered turn kept its decision_id and every decision was answered once
            ids = [a["decision_id"] for a in arena.acts]
            assert len(ids) == len(set(ids)) == 6
    arun(main())


def test_illegal_action_gets_error_and_retry():
    class Bad(Bot):
        strict = True
        n = 0
        errs = []
        def act(self, s):
            Bad.n += 1
            if Bad.n == 1:
                return Action.raise_to(5)          # below min_to -> bad_amount
            return Action.call() if s.legal.call else Action.check()
        def on_error(self, e): Bad.errs.append(e.code)

    async def main():
        async with MockArena(hands=1) as arena:
            await play(arena, Bad(), 1)
            assert arena.errors_sent == ["bad_amount"]
    arun(main())
    assert Bad.errs == ["bad_amount"] and Bad.n >= 3


def test_runner_legalizes_by_default():
    class Sloppy(Bot):
        def act(self, s):
            return Action.raise_to(5) if s.legal.raise_ else Action.check()

    async def main():
        async with MockArena(hands=2) as arena:
            await play(arena, Sloppy(), 2)
            assert arena.errors_sent == []
            assert arena.acts[0]["to"] == 200  # clamped to min_to
    arun(main())


def test_act_timeout_and_exception_fall_back_to_check_fold():
    class Slow(Bot):
        calls = 0
        async def act(self, s):
            Slow.calls += 1
            if Slow.calls == 1:
                await asyncio.sleep(5)
            raise RuntimeError("boom")

    async def main():
        async with MockArena(hands=1, decision_ms=500) as arena:
            await play(arena, Slow(), 1, safety_margin_ms=100)
            assert [a["type"] for a in arena.acts][0] == "fold"   # timed out -> fold (facing BB)
    arun(main())


def test_sync_blocking_act_hits_deadline():
    import time

    class Blocking(Bot):
        def act(self, s):
            time.sleep(2)
            return Action.call() if s.legal.call else Action.check()

    async def main():
        async with MockArena(hands=1, decision_ms=400) as arena:
            await play(arena, Blocking(), 1, safety_margin_ms=50)
            assert arena.acts[0]["type"] == "fold"
    arun(main())


def test_table_seq_gap_triggers_resync():
    async def main():
        async with MockArena(hands=2, emit_gap_at=6) as arena:
            await play(arena, CallingStation(), 2)
            hellos = [m for m in arena.received if m["type"] == "hello"]
            assert len(hellos) >= 2 and "resume" in hellos[1]["data"]
    arun(main())


def test_bad_token_rejected():
    async def main():
        async with MockArena() as arena:
            r = Runner(CallingStation(), arena.ws_url, "bad", reconnect=False)
            await r.run()
            assert r.connects == 1 and r.welcome is None
    arun(main())


def test_llm_bot_async_complete_end_to_end():
    async def complete(prompt):
        await asyncio.sleep(0)
        return "Strong hand.\nACTION: raise 300"

    async def main():
        async with MockArena(hands=2) as arena:
            bot = LLMBot(complete)
            await play(arena, bot, 2)
            assert arena.acts[0] == {"hand_id": "h-1", "decision_id": arena.acts[0]["decision_id"], "type": "raise", "to": 300}
    arun(main())


def test_webhook():
    body = json.dumps(DOC_YOUR_TURN).encode()
    st, out = handle_webhook(CallingStation(), body, sign("s3", body), "s3")
    assert st == 200 and json.loads(out) == {"hand_id": "h-77", "decision_id": "d-77-9", "type": "check"}
    st, _ = handle_webhook(CallingStation(), body, "sha256=bad", "s3")
    assert st == 401
    st, out = handle_webhook(CallingStation(), b'{"type":"action","data":{}}')
    assert st == 200 and out == b"{}"
