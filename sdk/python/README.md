# poker-sharingan (Python SDK)

PyPI distribution `poker-sharingan`; import package `sharingan`; CLI `sharingan`.

Bot SDK for the Poker Sharingan arena ([Table API](../../docs/TABLE_API.md)). Stdlib + `websockets`.

```bash
pip install "git+https://github.com/markokraemer/poker-sharingan#subdirectory=sdk/python"   # PyPI is coming; from a clone: pip install -e sdk/python
```

## Quickstart

```bash
pip install "git+https://github.com/markokraemer/poker-sharingan#subdirectory=sdk/python"   # PyPI is coming
```

```python
# mybot.py: the whole bot (or define a `Bot` subclass instead)
from sharingan import Action

def act(s):                                          # s = the your_turn payload (typed)
    if s.legal.raise_ and s.hole_cards[0][0] in "AK":
        return Action.raise_to(3 * s.big_blind)      # amounts are "to", auto-clamped to legal
    if s.legal.check:
        return Action.check()
    return Action.call() if s.pot_odds < 0.25 else Action.fold()
```

```bash
sharingan eval mybot.py      # ladder-v1 benchmark on the public arena -> one line:
# mybot: +132.4 ± 41.0 mbb/game (95% CI, aivat) over 3000 hands on ladder-v1 [unverified]
```

### CLI

| command | what it does |
|---|---|
| `sharingan eval mybot.py [--hands 3000] [--server URL] [--match]` | registers your bot and runs the `ladder-v1` benchmark track when the server has it (AIVAT-corrected; falls back to a 6-rotation duplicate match vs the house ladder on older servers, or with `--match`), prints ONE score line (mbb/game, 95% CI, hands). Default server: the public arena `https://arena.sharingan.poker` (or `$SHARINGAN_SERVER` / `--server` for a self-hosted one; a local one that is down prints the command to start it) |
| `sharingan benchmark mybot.py --track ladder-v1\|sharingan-1 [--hands N] [--server URL] [--token T]` | registers (or reuses `--token`, or the token saved for the bot), connects, starts `POST /v1/benchmarks/{track}/runs`, shows a live progress bar from `benchmark_progress`, then prints the final `mbb/game ± CI` and the leaderboard URL. The bot runs on your machine, so the run is `[unverified]` and never counts for the competition |
| `sharingan submit mybot.py [--track T] [--hands N] --contact EMAIL --profile URL` | hosted job: uploads the bot, our machines play it in a sandbox (Python, no network, 4 CPU, 12 GB) and print a `[VERIFIED, hosted job ...]` score. Only a full 20,004-hand run ranks; a shorter `--hands` (600 to 200,000) prints `[VERIFIED, NOT RANKED ...]`. The accepted line shows the hands the server will play and the sha256 of your upload. A `Bot` subclass keeps state only within one rotation of a deal (about 1/6 of a run), and hosted runs only fire `on_hand_end` / `on_error` |
| `sharingan token [mybot.py\|NAME]` | prints the saved bot token (stdout, nothing else) for that bot on the configured server, or the only saved token for the server: `curl -H "authorization: Bearer $(sharingan token mybot.py)" ...`. Exit 1 with a message on stderr if there is none or several |
| `sharingan connect mybot.py --server URL --token T [--player-id ID]` | connects an already-registered bot to `/v1/play` and keeps answering `your_turn` (reconnecting; `--no-reconnect` to exit when the server closes, `--max-hands N` to stop after N hands) |
| `sharingan play mybot.py --vs sharingan-1 [--server URL] [--table ID --seat N]` | creates (or joins) a table; house bots fill the other seats; prints each hand's result. Reuses the bot's saved token (no new registration per run). `--vs sharingan-1` plays the released blueprint without search |
| `sharingan serve mybot.py --port 9000 [--secret S]` | exposes the bot as a signed webhook endpoint (`POST /act`), so a bot can be just a URL. Webhook mode is disabled on arena.sharingan.poker; it is for self-hosted arenas |
| `sharingan pipe mybot.py` | serves the bot over stdin/stdout (newline-delimited JSON, TABLE_API.md section 14), for the match worker: `sharingan-arena run --bot "sharingan pipe mybot.py" --track ladder-v1 --lanes 8 --out result.json`. stdout is reserved for the protocol (your `print()`s go to stderr). Hooks: `on_connect`, `on_hand_end`, `on_error` (only if you override them); the benchmark pipe sends no `hand_start`/`action`/`street` |
| `sharingan eval mybot.py --local [--track T] [--hands N] [--lanes L] [--out result.json]` | no server: runs the `sharingan-arena` worker binary found on `PATH` (or `$SHARINGAN_ARENA_BIN`) with your bot as a pipe subprocess and prints the score line. Without the binary it prints how to build it (`cargo build --release -p sharingan-arena`) or to use `--server` |

`mybot.py` can define a `Bot` subclass, a module-level `bot` instance, or a plain `act(state)` function. The same file works unchanged over WebSocket (`connect`/`benchmark`), webhook (`serve`) and pipe (`pipe`). The bot is named after the file unless you set `name` yourself (also for `LLMBot`), and one name is one saved token and one leaderboard row.

### Programmatic version

```python
from sharingan import Action, ArenaClient, Bot, run

class MyBot(Bot):
    def act(self, s):
        return Action.check() if s.legal.check else Action.fold()

api = ArenaClient("https://arena.sharingan.poker")
me = api.register_bot("my-bot")                          # token shown once
api.join_table("TABLE_ID", seat=0, player_id=me.player_id)
run(MyBot(), "https://arena.sharingan.poker", me.token)  # connect, reconnect, answer every your_turn
```

## What you get

| | |
|---|---|
| `ArenaClient` | REST: `register_bot`, `create_table`, `join_table`, `leave_table`, `get_table`, `create_match`, `get_hands`, ... Raises `ApiException(status, code)` |
| `Runner` / `run()` | WebSocket: hello, `resume` after reconnect, 15 s ping, duplicate/gap detection (`table_seq`, `seq`), `decision_id` idempotency, re-answer after `illegal_action`/`bad_amount`, deadline guard |
| `Bot` | implement `act(state) -> Action` (sync or `async`); optional `on_connect/on_hand_start/on_action/on_street/on_hand_end/on_error` |
| `YourTurn` | typed `your_turn` with `legal`, `history`, `players`, `pots`, plus `.to_call`, `.pot_odds`, `.can_check`, `.min_raise_to`, `.max_raise_to`, `.stack`, `.me` |
| helpers | `pot_raise_to(s, 0.75)`, `bb_multiple_to(s, 3)`, `clamp_raise_to`, `legalize`, `safe_action`, `preflop_strength`, `render_state`, ... |
| webhook | `serve_webhook(bot, port=8000, secret=...)` or `handle_webhook(...)` for stateless bots (HMAC `X-Sharingan-Signature`; the older `X-Signature` is still accepted) |

### Behaviour worth knowing

* **"to", never "by".** `Action.raise_to(400)` means 400 total on this street. `bet` is an alias of `raise`.
* **Safe by default.** The runner passes every action through `legalize` (fold-when-free becomes check, a raise that is not allowed becomes call/check, amounts are clamped). Set `strict = True` on your bot to send exactly what you return.
* **Deadlines.** If `act` raises or exceeds the decision window (server clock from the envelope, minus `safety_margin_ms`), the runner plays check/fold, the same as the server's timeout policy. Sync `act` runs in a thread so a blocking LLM call cannot stall the connection.
* Chips are ints. Hand-end `hole_cards` keys are normalised to `int` seats.

## Example bots (`sharingan.bots`)

`RandomBot`, `CallingStation`, `TightAggressive` (rules), and `LLMBot(complete)`:

```python
from sharingan.bots import LLMBot
def complete(prompt: str) -> str: ...        # any provider, sync or async; you own the keys
run(LLMBot(complete), URL, token)
bot = LLMBot(complete)                       # in mybot.py for the CLI: named after the file; LLMBot(complete, name="x") to choose
```

An LLM bot needs the network, so it runs on your machine (`sharingan benchmark mybot.py`): the run is unverified and does not count for the competition.

`render_state(s)` gives the text the LLM sees; `LLMBot` parses the last `ACTION: ...` line.
See `examples/`.

## Testing your bot

`sharingan.mock.MockArena` is an in-process arena (REST + WebSocket) with scripted hands, error injection, forced disconnects and sequence gaps:

```python
async with MockArena(hands=3) as arena:
    await Runner(MyBot(), arena.ws_url, "any-token", max_hands=3).run()
    assert arena.errors_sent == []
```

Run the SDK's own suite: `pip install -e '.[dev]' && pytest`. Against a real arena (needs `ARENA_BLUEPRINT`): `SHARINGAN_E2E_SERVER=http://localhost:8080 pytest tests/test_e2e.py`.
