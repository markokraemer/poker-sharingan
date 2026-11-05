# Poker Sharingan

An open reproduction of [Pluribus](https://doi.org/10.1126/science.aay2400), the first AI to beat professionals at six-player no-limit hold'em, and an arena that scores poker bots with the luck removed.

[sharingan.poker](https://sharingan.poker) · [Play](https://sharingan.poker/play/) · [Leaderboard](https://sharingan.poker/leaderboard/) · [Docs](https://sharingan.poker/docs/) · [Weights](https://huggingface.co/markokraemer/sharingan-2) · [Learn](https://sharingan.poker/learn/)

## SHARINGAN-2

[**SHARINGAN-2**](https://huggingface.co/markokraemer/sharingan-2) is a from-scratch reproduction of Pluribus: a Monte Carlo CFR blueprint trained for 1.83e11 iterations, usable with optional turn and river search from the Rust code.

One training run, measured along the way:

| checkpoint | result |
|---|---|
| 2e10 | +77.6 bb/100 over 2,329 live hands on a public bot arena, with search |
| 4.77e10 | Beats SvanBot, the top bot on that arena, in both seatings |
| 1.83e11 (final) | +5.1 ± 1.2 mbb/game over the 2e10 checkpoint, +2.0 ± 1.0 over 4.77e10 (4M deals, 3 vs 3) |

In November 2026 an external bot found an action-translation leak: bets below the smallest menu size were read as checks, after which the blueprint lost track of the hand and called everything (+5,231 mbb/game without search). That leak is fixed in the Rust source; offline tests of that fixed build put the same bot at −390 ± 156 against final SHARINGAN-2 over 20,004 hands, and the six scripted sizing probes also lose. Hosted deployment results are reported separately.

A fresh hosted rerun of the exact original sombra submission on November 5 still wins **+4,761 ± 492 mbb/game** against the public C@2e10 track, verified over 20,004 hands with zero forfeits. A fresh fixed offline C@2e10 rerun scores −467 ± 162, with the earlier −420 ± 162 reference reproduced exactly. These are separate deployment/input paths and deal samples; the hosted result does not establish that the latest retained policy is deployed. [Run record](https://arena.sharingan.poker/v1/benchmarks/sharingan-1/runs/w_da35215b0163).

A frozen 3M-iteration attacker nevertheless wins **+2,466 ± 75 mbb/game against the fixed final SHARINGAN-2 blueprint**, confirmed on two deal seeds with verified model/learner hashes (1.2M hands, duplicate + AIVAT). The same policy wins **+2,307 ± 75 against verified C@2e10**, with both seed results exactly reproduced on a fresh public model copy. An earlier frozen 1M attacker also wins +714 ± 67 against final SHARINGAN-2. Standard local best response also reproduces **+841 ± 162 against verified C**. The original host's supplied model file has inconsistent reads; new evaluations use a separately verified private copy. These are attack bounds against five blueprint copies, without search. Causes and fixes remain under investigation; no six-player bot can be proven unexploitable. [Measurements and input audit](docs/RESULTS.md#action-translation-leak-and-fix-nov-2026).

Full numbers: [docs/RESULTS.md](docs/RESULTS.md) · Method: [docs/METHOD.md](docs/METHOD.md) · Model cards: [SHARINGAN-1 and SHARINGAN-2](model-cards/README.md)

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
sharingan submit mybot.py --track sharingan-1   # vs five copies of SHARINGAN-1/C@2e10
sharingan benchmark mybot.py                    # runs on your machine, unverified
```

A bot is one function, `act(state)`. It receives the decision and returns an action. Amounts are the total to bet to, not the amount added:

```json
→ {"hole_cards": ["As","Kd"], "board": ["7h","2c","9s"], "pot": 600,
   "legal": {"fold": true, "call": {"to": 200}, "raise": {"min_to": 400, "max_to": 9800}}}
← {"type": "raise", "to": 800}
```

Spec: [docs/TABLE_API.md](docs/TABLE_API.md) · SDKs: [sdk/](sdk/) · Agent skill: [skills/poker-sharingan](skills/poker-sharingan/SKILL.md)

The public `POST https://arena.sharingan.poker/v1/act` endpoint uses SHARINGAN-1/C@2e10 with turn/river search ([format](sharingan-1/serve/README.md)). The public `sharingan-1` benchmark track and browser table use C@2e10 without search. To use the final SHARINGAN-2 weights, self-host with `--blueprint weights/sharingan-2.plcp`. Model weights and the deployed Rust policy version are separate; use the current evaluation records when comparing results.

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
