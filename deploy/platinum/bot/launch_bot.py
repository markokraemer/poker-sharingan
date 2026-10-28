"""Connect a user-uploaded Python bot to the arena and play until stopped. UNTESTED.

env: BOT_FILE (path to bot .py), ARENA_URL (https://arena.sharingan.poker), ARENA_TOKEN (bot token), MAX_HANDS (optional)
The runner service starts the benchmark run (POST /v1/benchmarks/{track}/runs) after this process is connected;
the arena answers 409 not_connected until the websocket is up, and the runner retries.

Why not `sharingan play` / `sharingan serve`:
  * `play` creates its own table against house bots; a benchmark run needs the bot merely CONNECTED to /v1/play.
  * `serve` is webhook mode (arena -> bot HTTP). Needs ARENA_ENABLE_WEBHOOKS=true and a public URL per sandbox, and adds
    a CP/edge hop per decision. Connect mode is outbound-only and the fastest path.
There is no `sharingan connect` CLI yet; this file is that command.
"""
import asyncio
import logging
import os
import signal
import sys

from sharingan.cli import load_bot
from sharingan.client import Runner

logging.basicConfig(level=os.environ.get("LOG_LEVEL", "WARNING"), stream=sys.stderr)


async def main() -> int:
    bot = load_bot(os.environ["BOT_FILE"])
    max_hands = int(os.environ.get("MAX_HANDS", "0")) or None
    runner = Runner(bot, os.environ["ARENA_URL"], os.environ["ARENA_TOKEN"], max_hands=max_hands)
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(sig, runner.stop)
    print("sharingan-bot: connecting", flush=True)
    await runner.run()
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
