# Poker Sharingan

An open reproduction of [Pluribus](https://doi.org/10.1126/science.aay2400), the first AI to beat professionals at six-player no-limit hold'em, and an arena that scores poker bots with the luck removed.

[sharingan.poker](https://sharingan.poker) · [Play](https://sharingan.poker/play/) · [Leaderboard](https://sharingan.poker/leaderboard/) · [Docs](https://sharingan.poker/docs/) · [Weights](https://huggingface.co/markokraemer/sharingan-2) · [Learn](https://sharingan.poker/learn/)

## SHARINGAN-2

[**SHARINGAN-2**](https://huggingface.co/markokraemer/sharingan-2) is a from-scratch reproduction of Pluribus: a Monte Carlo CFR blueprint trained for 1.83e11 iterations, played with real-time search on the turn and river.

One training run, measured along the way:

| checkpoint | result |
|---|---|
| 2e10 | +77.6 bb/100 over 2,329 live hands on a public bot arena, with search |
| 4.77e10 | Beats SvanBot, the top bot on that arena, in both seatings |
| 1.83e11 (final) | +5.1 ± 1.2 mbb/game over the 2e10 checkpoint, +2.0 ± 1.0 over 4.77e10 (4M deals, 3 vs 3) |

In November 2026 a competition entry found an action-translation leak: bets below the smallest menu size were read as checks, after which the blueprint lost track of the hand and called everything (+5,231 mbb/game against the blueprint without search). It is fixed; the same bot now loses −390 ± 156 against SHARINGAN-2, and adversarial bet-size probes lose across the board. Details: [docs/RESULTS.md](docs/RESULTS.md#action-translation-leak-and-fix-nov-2026). A larger adversarial study finds a frozen trained attacker winning +457 ± 67 mbb/game against the fixed 2e10 checkpoint (200k duplicate deals per seat). Standard local best response also finds +841 ± 162 (20k duplicate deals per seat, raw winnings). These are profitable deviations against five copies of that checkpoint; their causes are under investigation, with final-model confirmation pending. No six-player bot can be proven unexploitable.

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
sharingan submit mybot.py --track sharingan-1   # vs five copies of SHARINGAN-2 (2e10 checkpoint)
sharingan benchmark mybot.py                    # runs on your machine, unverified
```

A bot is one function, `act(state)`. It receives the decision and returns an action. Amounts are the total to bet to, not the amount added:

```json
→ {"hole_cards": ["As","Kd"], "board": ["7h","2c","9s"], "pot": 600,
   "legal": {"fold": true, "call": {"to": 200}, "raise": {"min_to": 400, "max_to": 9800}}}
← {"type": "raise", "to": 800}
```

Spec: [docs/TABLE_API.md](docs/TABLE_API.md) · SDKs: [sdk/](sdk/) · Agent skill: [skills/poker-sharingan](skills/poker-sharingan/SKILL.md)

To get a decision from SHARINGAN-2 in your own code, call `POST https://arena.sharingan.poker/v1/act` with your bot token ([format](sharingan-1/serve/README.md)). It serves the 2e10 checkpoint with search.

## Competition

Bots that beat SHARINGAN-2 (its 2e10 checkpoint, track `sharingan-1`) on the leaderboard by November 15, 2026 (23:59 UTC) qualify. The leading bots are re-run on fresh deals, and the top six play each other at a final table on November 16. Rules: [COMPETITION.md](COMPETITION.md).

## Repository

```
engine/        poker rules
arena/         harness, scoring, reference bots, arena server
sharingan-1/   SHARINGAN-2 source: training, real-time search, inference server
sdk/           Python and TypeScript SDKs
skills/        agent skill
deploy/        production deployment
research/      small-game labs and experiments
docs/          specs, results, method
site/          sharingan.poker
```

Self-host the arena: [arena/server](arena/server/README.md#self-host) · Train from scratch: [docs/TRAINING.md](docs/TRAINING.md) · Deployment: [deploy/production](deploy/production/README.md)

Apache-2.0. For research and bot arenas; real-money sites ban bots.
