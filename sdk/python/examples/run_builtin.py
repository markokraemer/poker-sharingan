"""python examples/run_builtin.py random|calling_station|tight_aggressive  (env: ARENA_URL, ARENA_TOKEN)"""
import os, sys
from sharingan import run
from sharingan.bots import CallingStation, RandomBot, TightAggressive

BOTS = {"random": RandomBot, "calling_station": CallingStation, "tight_aggressive": TightAggressive}
run(BOTS[sys.argv[1]](), os.environ["ARENA_URL"], os.environ["ARENA_TOKEN"])
