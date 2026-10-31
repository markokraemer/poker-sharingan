# Poker Sharingan

1. **SHARINGAN-1**, the #1 open-source poker bot.
2. **The arena** to build a better one, and prove it.

[sharingan.poker](https://sharingan.poker)

In poker, luck hides skill for thousands of hands. We replay every deal from every seat and subtract the luck, so the score actually means something.

## Competition: $5,000 in prizes, final table November 16

Beat SHARINGAN-1 to qualify on the [leaderboard](https://sharingan.poker/leaderboard/) by November 15. The top 6 meet at the final table on November 16 and play each other. $5,000 in prizes. Enter with `sharingan submit mybot.py --track sharingan-1 --contact you@example.com --profile https://x.com/you`. Rules and terms: [COMPETITION.md](COMPETITION.md). Sponsors welcome: DM [@markokraemer](https://x.com/markokraemer).

## Open source (this repo)

| | |
|---|---|
| **Engine** | Six-player no-limit hold'em. `engine/` |
| **Harness** | Run millions of hands locally. Luck-corrected scores with error bars. `arena/` |
| **SHARINGAN-1** | Our bot, a Pluribus reproduction. Code here, weights on [Hugging Face](https://huggingface.co/markokraemer/sharingan-1). `sharingan-1/` |

## Hosted (free during beta)

We host the arena: your bot plays the field at scale on our machines, you get a verified, ranked score and every hand history. Paid credits for large volumes come later.

## Your bot's API

Your bot gets the decision. It returns an action. Amounts are the total "to", never "by".

```json
→ {"type": "your_turn", "data": {"hole_cards": ["As","Kd"], "board": ["7h","2c","9s"], "pot": 600,
   "legal": {"fold": true, "call": {"to": 200}, "raise": {"min_to": 400, "max_to": 9800}}, ...}}
← {"type": "act", "data": {"decision_id": "...", "type": "raise", "to": 800}}
```

Full spec: [docs/TABLE_API.md](docs/TABLE_API.md). The SDK hides all of this behind `act(state)`.

## SHARINGAN-1

A from-scratch reproduction of [Pluribus](https://doi.org/10.1126/science.aay2400), the first AI to beat poker pros at six-player no-limit.
SHARINGAN-1 is blueprint C@2e10 with turn and river search, the bot behind these results:

- won +77.6 bb/100 over 2,329 live hands on a public bot arena
- a best response trained to exploit it, with the same bet sizes, still loses to it
- a longer-trained checkpoint (C@4.77e10, also released) beats SvanBot, the #1 bot on that arena, in both seatings

The leaderboard's sharingan-1 track and the inference API play its released blueprint, without real-time search.

Weights: [huggingface.co/markokraemer/sharingan-1](https://huggingface.co/markokraemer/sharingan-1) ([model card](MODEL_CARD.md)) · Details: [docs/RESULTS.md](docs/RESULTS.md) · How it works: [docs/METHOD.md](docs/METHOD.md)

SHARINGAN-2, from the full-length training run, is next.

## Run it

With an agent, one prompt:

```
Read https://sharingan.poker/skill.md, then build a poker bot and get it on the leaderboard.
```

The same file is an Agent Skill in [skills/poker-sharingan](skills/poker-sharingan/SKILL.md). By hand:

```bash
pip install "git+https://github.com/markokraemer/poker-sharingan#subdirectory=sdk/python"   # PyPI is coming
export SHARINGAN_SERVER=https://arena.sharingan.poker
sharingan submit mybot.py --contact you@example.com --profile https://x.com/you   # our machines play 20,000 hands in ~10 s: VERIFIED score on the leaderboard
sharingan submit mybot.py --track sharingan-1     # the hard test: five copies of SHARINGAN-1 (contact + profile are saved after the first submit)
sharingan benchmark mybot.py                      # same, but the bot runs on your machine (LLM bots, network access)
```

Play SHARINGAN-1 live at [sharingan.poker/play](https://sharingan.poker/play), see the [leaderboard](https://sharingan.poker/leaderboard). The public arena is `https://arena.sharingan.poker` (REST, WebSocket `/v1/play`, hosted jobs `/v1/jobs`); how it is deployed: [deploy/production](deploy/production/README.md).

Ask SHARINGAN-1 for a decision from your own code: `POST https://arena.sharingan.poker/v1/act` with your bot token ([format](sharingan-1/serve/README.md)).

`mybot.py` is just an `act(state)` function. SDKs: [sdk/](sdk/) · Self-host the arena: [arena/server](arena/server/README.md#self-host) · Train SHARINGAN-1: [docs/TRAINING.md](docs/TRAINING.md)

## Layout

```
engine/        poker rules
arena/         harness, scoring, reference bots, adapters, the arena server
sharingan-1/   our bot: training, real-time search, inference server
sdk/           Python and TypeScript bot SDKs
skills/        the agent skill (same as sharingan.poker/skill.md)
deploy/        how sharingan.poker runs (containers, runner, edge)
research/      small-game labs and experiments
docs/          specs, results, method
site/          sharingan.poker
```

Apache-2.0 · For research and bot arenas. Real-money sites ban bots.
