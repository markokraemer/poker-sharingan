---
license: apache-2.0
library_name: custom
language:
- en
tags:
- poker
- game-theory
- cfr
- mccfr
- reinforcement-learning
- imperfect-information-games
- no-limit-holdem
- rust
pipeline_tag: reinforcement-learning
model-index:
- name: SHARINGAN-1
  results: []
---

# SHARINGAN-1: the Poker Sharingan house bot

SHARINGAN-1 is the reference bot at the top of the [Poker Sharingan](https://sharingan.poker) arena: an open reproduction of Pluribus (Brown & Sandholm, *Science* 2019) for 6-player no-limit Texas hold'em. This repository holds the
**trained strategy files** and the **card abstraction tables**. Code, training and evaluation tooling: https://github.com/markokraemer/poker-sharingan
(Rust workspace, "library: custom": there is no `transformers`/`pytorch` loader; the weights are read by the Rust crates `blueprint` and `bench`).

> TODO(final): this card describes the checkpoints evaluated so far. Fill in the final-run row of every table once the full-budget run is evaluated and uploaded.

## What is in this repository

| file | format | size | what it is |
|---|---|---|---|
| `buckets-200.tar` | raw u16 tables + `meta.txt` | ~0.28 GB | 200-bucket card abstraction per postflop street (extract to `data/buckets-200/`). Required to use any blueprint. |
| `search-buckets-500.tar` | raw tables | ~6.4 GB | optional per-flop 500-bucket search abstraction (built, unused by the final config) |
| `C-medium-4.77e10.plcp` | PLCP1 compact | ~142 GB | blueprint C after 4.77e10 iterations: strongest evaluated compact file |
| `C-medium-2e10.plcp` | PLCP1 compact | ~125 GB | blueprint C after 2e10 iterations (the bot that played live) |
| `B-local-2e10.plcp` | PLCP1 compact | ~28 GB | blueprint B (smaller `local` menu), the lightest option |
| `A-paper-2e10.plcp` | PLCP1 compact | ~187 GB | blueprint A (paper-style menu) |
| `C-medium-5.55e10/` | PLBP4 full checkpoint + `snapshots.sum` (PLSA1) | ~338 GB + 59 GB | latest C training state; resumable |
| `C-medium-final.plcp` | PLCP1 compact | TODO(final) | final full-budget checkpoint |
| `C-medium-final/` | PLBP4 | TODO(final) | final full training state |

Not all rows may be uploaded in the first release; check the file list of this repository. Sizes are from the training archive; intermediate
and raw checkpoints (A@3.43e10 at 17.0B infosets, 476 GB + 85 GB, and several earlier C states) exist but are not necessarily published.

### Formats

* **PLCP1 (compact policy)**: the playable average strategy only. 8-bit action probabilities plus an infoset index, **about 12 bytes per infoset**.
  Produced by `pluribus compact`. Read by `bench`, `openpoker`, `search` (`--policy compact:...` / `search:...`).
* **PLBP4 (full checkpoint)**: `blueprint.bin` with the full regrets and average strategy, enough to **resume training** (`pluribus train --resume`).
  Companion `snapshots.sum` (PLSA1) holds the postflop average-strategy snapshots. Checkpoints are written tmp-then-rename.
* **Bucket tables**: `flop.u16`, `turn.u16`, `river.u16` map a suit-isomorphic hand index to a bucket id; `meta.txt` records `buckets 200 200 200`.
  Preflop is lossless (169 classes).

Infoset counts: C reached 12.2B infosets (78% of a 460 GB store); A reached 17.0B (store full).

## Model details

* **Game:** 6-max (2-6 handled by the engine) no-limit hold'em, 100 bb stacks (blueprint). A separate 200 bb heads-up blueprint exists for Slumbot tests and is not part of this release.
* **Algorithm:** parallel external-sampling **Linear MCCFR** with negative-regret pruning. LCFR discount for the first 3.5% of the run, pruning from 1.7%
  (95% of iterations, threshold -300M, not on river or terminal actions), averaging from 6.9%, strategy interval 10k. The paper's minute-based schedule is scaled to iterations.
* **Abstraction:** preflop lossless; 200 buckets per postflop street (EMD k-means on equity histograms for flop/turn, OCHS for river). Action menus:
  `local` (B), `paper`-style (A), `medium` (C). **Menu must match at load time** (`--sizing`): evaluating a blueprint with the wrong menu gives garbage (about -2,364 mbb/game).
* **Real-time search (code, not weights):** depth-limited MC-LCFR with 4 continuation strategies at the leaves, Bayesian beliefs, off-tree re-search; final config searches turn and river only (`--set flop=0`), 200k iterations.
* **Action translation:** shadow-game tracking of the abstract state against the real one.
* **Seeds:** 2019 for C and A.

## Training

| run | menu | iterations reached | wall | notes |
|---|---|---|---|---|
| B | `local` | 2e10 (finished) | ~9.5 h | |
| C | `medium` | 5.55e10 (stopped) | ~58 h | 180k -> 355k it/s; still improving |
| A | `paper` | 3.43e10 (stopped) | ~66 h | store full at 17.0B infosets |
| **C final** | `medium` | TODO(final) | TODO(final) | target 1.83e11 iterations (about 8 days of total compute, matching the paper) |

Hardware: 64-core / 576 GB bare-metal servers, about EUR 1.86/h each; total compute spend so far about EUR 480. TODO(final): add the final run's hours and cost.

## Evaluation

All numbers: 6-max, duplicate deals + AIVAT, mbb/game (1 mbb/game = 0.1 bb/100), 95% CI. Full tables and caveats are in the GitHub README.

| test | result |
|---|---|
| C@2e10 vs B-final (3+3, 4M deals/format) | +1.6 +- 1.3 |
| C@4.77e10 vs C@2e10 (3+3) | +3.8 +- 1.0 |
| A@2e10 vs C@2e10 (3+3) | -4.0 +- 1.3 |
| trained best response (12M iters) vs C@2e10 / C@4.77e10 / A@2e10 | -323 +- 62 / -290 +- 59 / -372 +- 61 (the exploiter loses) |
| search (turn+river) vs 5 x C@2e10 | +87 +- 27 |
| search (turn+river) vs rule bots chart6 / loose / tight | +59 / +108 / +66 |
| Open Poker live, final config, 2,329 hands | +77.6 bb/100, 95% CI [26, 129] (all-in adjusted) |
| offline vs SvanBot at its live budget: 1 SvanBot vs 5 x blueprint / 1 blueprint vs 5 x SvanBot | SvanBot -46.9 +- 21.2 / blueprint +56.7 +- 34.7 |
| **C@final vs C@4.77e10** | TODO(final) |
| **C@final exploiter** | TODO(final) |

## Intended use

Research on imperfect-information game solving, abstraction and real-time search; benchmarking other poker agents; studying near-equilibrium
6-player play; playing against the bot locally or on arenas that explicitly allow bots.

## Limitations and risks

* **Not evaluated against strong humans** or against other top solvers; external references are one bot arena and one offline bot, both with caveats.
* **Exploitability is only bounded from below** by one trained best response limited by its abstraction and 12M iterations. Search-augmented play was not tested for exploitability.
* Trained for 100 bb stacks; very deep stacks need an external stack-reset guard (`--max-stack-bb`) or the strategy is out of distribution.
* Off-menu bet sizes are handled by action translation; sizes far from the menu are the main exploitable surface in this class of agents.
* A near-equilibrium bot is a weaker **exploiter** of weak opponents than adaptive bots; do not infer ladder performance from equilibrium strength.
* Files are very large and need correspondingly large disk/RAM; use B (28 GB) or a `--min-visits` pruned compact for constrained machines.
* **Dual use:** a strong poker agent can be used to cheat on real-money sites, which prohibit bots. Do not use it for that.

## Citation

```bibtex
@article{brown2019superhuman,
  title   = {Superhuman AI for multiplayer poker},
  author  = {Brown, Noam and Sandholm, Tuomas},
  journal = {Science}, volume = {365}, number = {6456}, pages = {885--890}, year = {2019}
}
```
