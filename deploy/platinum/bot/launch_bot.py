"""Connect a user-uploaded Python bot to the arena and play until stopped. Thin wrapper over `sharingan connect`.

env: BOT_FILE (path to bot .py), ARENA_URL (https://arena.sharingan.poker), ARENA_TOKEN (bot token), MAX_HANDS (optional),
     LOG_LEVEL (optional)
Connect mode only (worker mode uses `sharingan pipe` inside `sharingan-arena run`).
The runner service starts the benchmark run (POST /v1/benchmarks/{track}/runs, runner token, hosted:true) after this process is connected;
the arena answers 409 not_connected until the websocket is up, and the runner retries.

Why `connect` and not `play` / `serve`: `play` creates its own table against house bots, `serve` is webhook mode
(arena -> bot HTTP, needs a public URL per sandbox). Connect mode is outbound-only and the fastest path.
"""
import logging
import os
import sys

from sharingan.cli import main

logging.basicConfig(level=os.environ.get("LOG_LEVEL", "WARNING"), stream=sys.stderr)


def argv_from_env() -> list:
    argv = ["connect", os.environ["BOT_FILE"], "--server", os.environ["ARENA_URL"], "--token", os.environ["ARENA_TOKEN"]]
    if int(os.environ.get("MAX_HANDS", "0") or 0):
        argv += ["--max-hands", os.environ["MAX_HANDS"]]
    return argv


if __name__ == "__main__":
    sys.exit(main(argv_from_env()))
