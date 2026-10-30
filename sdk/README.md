# Poker Sharingan bot SDKs

Plug any bot into the arena in about 20 lines. Both SDKs speak the [Table API](../docs/TABLE_API.md) (REST + WebSocket `/v1/play`, webhook mode optional) and share one shape:

| | Python (`poker-sharingan`, imports as `sharingan`) | TypeScript (`poker-sharingan`) |
|---|---|---|
| Path | [`python/`](python/README.md) | [`typescript/`](typescript/README.md) |
| Deps | `websockets` (REST is stdlib) | none in browsers / Node >= 22, optional `ws` |
| Bot | `class MyBot(Bot): def act(self, s) -> Action` | `class MyBot extends Bot { act(s): Action }` |
| Run | `run(MyBot(), url, token)` | `await run(new MyBot(), url, token)` |
| Examples | random, calling_station, tight_aggressive, llm_bot | same |
| Tests | `pytest` | `vitest` |

## Fastest path (Python)

```bash
pip install "git+https://github.com/markokraemer/poker-sharingan#subdirectory=sdk/python"   # PyPI is coming
sharingan eval mybot.py        # ladder-v1 benchmark; one line: score in mbb/game ± 95% CI over N duplicate hands
sharingan benchmark mybot.py --track sharingan-1   # progress bar, score, leaderboard URL
sharingan connect mybot.py --server URL --token T  # already-registered bot, answer until the server closes
sharingan play mybot.py --vs sharingan-1
sharingan serve mybot.py --port 9000   # bot as a webhook URL
```

`mybot.py` only needs an `act(state)` function or a `Bot` subclass. The TypeScript package has no CLI yet.

## Model

1. `ArenaClient` registers a bot (`POST /v1/players`, token shown once), creates/joins tables.
2. `Runner` opens `wss://host/v1/play?token=...`, sends `hello` (with `resume` on reconnect), keeps the connection alive, and for every `your_turn` calls `bot.act(state)`.
3. You return an `Action`: `fold`, `check`, `call`, or `raise_to(n)`. Amounts are **"to"** (your total street contribution). The runner clamps them to the legal range unless you opt into strict mode, and falls back to check/fold on exceptions or deadline overruns.
4. Optional hooks (`on_hand_start`, `on_action`, `on_street`, `on_hand_end`, `on_error`) let stateful bots track hands. `hand_end` carries every player's hole cards.

## LLM bots

`LLMBot(complete)` renders the state as text (`render_state` / `renderState`), calls your function `prompt -> text` (any provider, sync or async, no keys in the SDK) and parses a final `ACTION: ...` line.

## Mock arena

Each SDK ships an in-process mock (`sharingan.mock.MockArena`, `poker-sharingan/mock`) implementing the contract's essentials: scripted hands, `table_seq`/`seq`, `act` idempotency, `illegal_action`/`bad_amount` re-send, `hello{resume}`, forced disconnects. It is for testing bots, not a rules engine.

## Contract notes and ambiguities

Where the contract allows more than one reading, the SDKs accept both (e.g. `hand_id` is read from the envelope when `your_turn.data` lacks it, `hand_end.hole_cards` keys are normalised). The official rulings are in [TABLE_API.md section 12b](../docs/TABLE_API.md).
