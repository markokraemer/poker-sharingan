# Poker Sharingan

An open reproduction of [Pluribus](https://doi.org/10.1126/science.aay2400), the first AI to beat professionals at six-player no-limit hold'em, and an arena that scores poker bots with the luck removed.

[sharingan.poker](https://sharingan.poker) · [Play](https://sharingan.poker/play/) · [Leaderboard](https://sharingan.poker/leaderboard/) · [Docs](https://sharingan.poker/docs/) · [Weights](https://huggingface.co/markokraemer/sharingan-1)

## Models

| | |
|---|---|
| [**SHARINGAN-1**](https://huggingface.co/markokraemer/sharingan-1) | Blueprint at 2e10 iterations, played with turn and river search. The leaderboard plays the blueprint alone, without search. |
| [**SHARINGAN-2**](https://huggingface.co/markokraemer/sharingan-2) | The same run trained to the full 1.83e11 iterations. Beats SHARINGAN-1 by +5.1 ± 1.2 mbb/game. |

Results:

- **+77.6 bb/100** over 2,329 live hands on a public bot arena, with search.
- A longer-trained checkpoint (C@4.77e10) beats SvanBot, the top bot on that arena, in both seatings.
- A short-trained best response found no profitable exploit of the blueprint. This is a sanity check, not a measure of exploitability.

Full numbers: [docs/RESULTS.md](docs/RESULTS.md) · Method: [docs/METHOD.md](docs/METHOD.md) · Model card: [MODEL_CARD.md](MODEL_CARD.md)

## The arena

Over a few thousand hands, luck hides skill. The arena replays every deal from every seat and subtracts the luck, so a score reflects how the bot plays.

- **Engine:** six-player no-limit hold'em (`engine/`).
- **Harness:** millions of hands on your own machine, with luck-corrected scores and error bars (`arena/`).
- **Hosted runs:** submit a bot and our machines play 20,004 hands. You get a verified score on the leaderboard and the deal seed to replay the run.

## Quickstart

Give your coding agent one line:

```
Read https://sharingan.poker/skill.md, then build a poker bot and get it on the leaderboard.
```

Or by hand:

```bash
pip install "git+https://github.com/markokraemer/poker-sharingan#subdirectory=sdk/python"
export SHARINGAN_SERVER=https://arena.sharingan.poker

sharingan submit mybot.py --contact you@example.com --profile https://github.com/you
sharingan submit mybot.py --track sharingan-1   # five copies of SHARINGAN-1
sharingan benchmark mybot.py                    # runs on your machine, unverified
```

A bot is one function, `act(state)`. It receives the decision and returns an action. Amounts are the total to bet to, not the amount added:

```json
→ {"hole_cards": ["As","Kd"], "board": ["7h","2c","9s"], "pot": 600,
   "legal": {"fold": true, "call": {"to": 200}, "raise": {"min_to": 400, "max_to": 9800}}}
← {"type": "raise", "to": 800}
```

Spec: [docs/TABLE_API.md](docs/TABLE_API.md) · SDKs: [sdk/](sdk/) · Agent skill: [skills/poker-sharingan](skills/poker-sharingan/SKILL.md)

To get a decision from SHARINGAN-1 in your own code, call `POST https://arena.sharingan.poker/v1/act` with your bot token ([format](sharingan-1/serve/README.md)). It plays with search.

## Competition

Bots that beat SHARINGAN-1 on the leaderboard by November 15, 2026 (23:59 UTC) qualify. The leading bots are re-run on fresh deals, and the top six play each other at a final table on November 16. Rules: [COMPETITION.md](COMPETITION.md).

## Repository

```
engine/        poker rules
arena/         harness, scoring, reference bots, arena server
sharingan-1/   training, real-time search, inference server
sdk/           Python and TypeScript SDKs
skills/        agent skill
deploy/        production deployment
research/      small-game labs and experiments
docs/          specs, results, method
site/          sharingan.poker
```

Self-host the arena: [arena/server](arena/server/README.md#self-host) · Train from scratch: [docs/TRAINING.md](docs/TRAINING.md) · Deployment: [deploy/production](deploy/production/README.md)

Apache-2.0. For research and bot arenas; real-money sites ban bots.
