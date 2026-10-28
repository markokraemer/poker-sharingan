"""Throughput probe: one external WebSocket bot (python SDK) vs 5 house bots in a duplicate match.

    PYTHONPATH=sdk/python:<websockets> python3 arena/server/scripts/throughput.py \
        --server http://127.0.0.1:8080 --hands 3000 --mode benchmark --lanes 1

Prints hands/s measured from match start to `finished` (includes the server-side AIVAT pass)
and from start to the last played hand (play only).
"""
import argparse, asyncio, time

from sharingan.bot import Bot
from sharingan.client import ArenaClient, Runner
from sharingan.models import Action


class Caller(Bot):
    name = "caller"

    def act(self, state):
        return Action.check() if state.legal.check else Action.call()


async def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--server", default="http://127.0.0.1:8080")
    ap.add_argument("--hands", type=int, default=3000)
    ap.add_argument("--mode", default="normal")
    ap.add_argument("--lanes", type=int, default=1)
    ap.add_argument("--vs", default="tight_aggressive")
    ap.add_argument("--aivat", default="full")
    a = ap.parse_args()
    api = ArenaClient(a.server)
    me = api.register_bot("tp-caller")
    runner = Runner(Caller(), a.server, me.token)
    rt = asyncio.ensure_future(runner.run())
    await asyncio.sleep(0.5)
    cfg = {"decision_time_ms": 5000, "mode": a.mode}
    body = dict(table_config=cfg, participants=[me.player_id] + [f"house:{a.vs}"] * 5, hands_planned=a.hands,
                duplicate={"enabled": True, "rotations": 6, "deck_from": "seed"}, seed="tp")
    body["aivat"] = a.aivat
    if a.lanes > 1:
        body["lanes"] = a.lanes
    m = api.request("POST", "/v1/matches", body)
    api.match_action(m["match_id"], "start")
    t0 = time.time()
    t_play = None
    while True:
        cur = api.get_match(m["match_id"])
        if t_play is None and cur["hands_played"] >= a.hands:
            t_play = time.time() - t0
        if cur["status"] in ("finished", "aborted"):
            break
        await asyncio.sleep(0.1)
    t1 = time.time() - t0
    runner.stop()
    await asyncio.gather(rt, return_exceptions=True)
    tp = t_play or t1
    print(f"mode={a.mode} lanes={a.lanes} vs={a.vs} hands={a.hands}: play {tp:.2f}s = {a.hands / tp:.1f} hands/s; "
          f"total incl. finalize {t1:.2f}s = {a.hands / t1:.1f} hands/s; status={cur['status']}")


asyncio.run(main())
