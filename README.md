# Poker Sharingan

**The arena for poker bots.** Bring a bot, get an honest score. [sharingan.poker](https://sharingan.poker)

In poker, luck hides skill for thousands of hands. We replay every deal from every seat and subtract the luck, so the score actually means something.

## Open source (this repo)

| | |
|---|---|
| **Engine** | Six-player no-limit hold'em. `engine/` |
| **Harness** | Run millions of hands locally. Luck-corrected scores with error bars. `arena/` |
| **SHARINGAN-1** | Our bot, a Pluribus reproduction. Code and weights. `sharingan-1/` |

## Cloud (paid, coming soon)

Buy credits, run evals. We host the arena and the API: your bot plays the field at scale, you get a ranked score and every hand history.

## Your bot's API

Your bot gets the hand. It returns an action.

```json
→ {"hand": ["As","Kd"], "board": ["7h","2c","9s"], "pot": 600, "to_call": 200, "options": ["fold","call","raise"]}
← {"action": "raise", "to": 800}
```

Full spec: [docs/TABLE_API.md](docs/TABLE_API.md)

## SHARINGAN-1

A from-scratch reproduction of [Pluribus](https://doi.org/10.1126/science.aay2400), the first AI to beat poker pros at six-player no-limit. Its final training run is finishing now. Earlier checkpoints:

- beat SvanBot, the #1 bot on a public arena
- a bot trained only to exploit it still loses to it
- won +77.6 bb/100 over 2,329 live hands

Details: [docs/RESULTS.md](docs/RESULTS.md) · How it works: [docs/METHOD.md](docs/METHOD.md)

## Run it

```bash
cargo build --release
./target/release/pluribus h2h --a bot1.plcp --b bot2.plcp --players 6 --deals 1000000 ...
```

More: [docs/OVERVIEW.md](docs/OVERVIEW.md)

## Layout

```
engine/        poker rules
arena/         harness, scoring, reference bots, adapters
sharingan-1/   our bot: training and real-time search
research/      small-game labs and experiments
docs/          specs, results, method
site/          sharingan.poker
```

Apache-2.0 · For research and bot arenas. Real-money sites ban bots.
