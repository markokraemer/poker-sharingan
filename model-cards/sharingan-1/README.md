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

# SHARINGAN-1: the C@2e10 Poker Sharingan blueprint

> This is C@2e10, the earlier checkpoint of the training run that produced [SHARINGAN-2](https://huggingface.co/markokraemer/sharingan-2) at 1.83e11 iterations. The [SHARINGAN-2 model card](https://huggingface.co/markokraemer/sharingan-2) describes the final weights. Both releases have confirmed profitable deviations; longer training is not a robustness guarantee.

SHARINGAN-1 is the reference bot of the [Poker Sharingan](https://sharingan.poker) arena: an open reproduction of Pluribus (Brown & Sandholm, *Science* 2019) for 6-player no-limit Texas hold'em. This repository holds the
**trained strategy files** and the **card abstraction tables**. Code, training and evaluation tooling: https://github.com/markokraemer/poker-sharingan
(Rust workspace, "library: custom": there is no `transformers`/`pytorch` loader; the weights are read by the Rust crates `blueprint` and `bench`).

The released SHARINGAN-1 weights are blueprint **C@2e10** (`medium` bet sizes). The search-augmented SHARINGAN-1 agent adds optional turn/river search from the Rust code; search is not stored in these weights. The documented public configurations differ:
the inference API (`/v1/act`) plays the full bot (it falls back to the blueprint when a search slot is busy); the leaderboard's `sharingan-1` track,
`sharingan play --vs sharingan-1` and the browser play page use the released blueprint **without** search. The full-budget run (1.83e11 iterations)
is released as [SHARINGAN-2](https://huggingface.co/markokraemer/sharingan-2): its blueprint beats C@2e10 by +5.09 ± 1.16 mbb/game (3 vs 3, 4M duplicate deals). The public track ID stays `sharingan-1`; it uses C@2e10, not the final weights.

**Evaluation provenance matters:** model weights, action-translation code and deployed worker versions are separate. The source-level sync fix and offline attack scores do not establish which policy version is deployed. Record the full model checksum, buckets, sizing, code revision and tracking options; the public blueprint fingerprint alone does not establish all of them. See the [current results and deployment checks](https://github.com/markokraemer/poker-sharingan/blob/main/docs/RESULTS.md#action-translation-leak-and-fix-nov-2026).

**Fixed offline rerun (2026-11-05):** the original tiny-bet attacker scores −467 ± 162 against verified C@2e10 over 20,004 hands, with zero forfeits. The earlier −420 ± 162 fixed reference reproduced exactly. Operational hosted-run receipts are private.

Public play, hosted benchmarks and standings are temporarily unavailable. The weights and local/self-hosted tooling remain available.

## What is in this repository

| file | size | what it is |
|---|---|---|
| `sharingan-1.plcp.part-*` | 124.7 GB | **SHARINGAN-1**: blueprint C after 2e10 iterations (PLCP1 compact). The blueprint of the bot that played live (with search on top). The arena's leaderboard track and play page run it without search |
| `C-medium-4.77e10.plcp.part-*` | 141.7 GB | earlier blueprint C checkpoint after 4.77e10 iterations (+3.8 ± 1.0 mbb/game vs C@2e10; the final SHARINGAN-2 is stronger in the reported head-to-head) |
| `B-local-2e10.plcp.part-*` | 28.3 GB | blueprint B (smaller `local` menu): the light option for normal machines |
| `A-paper-2e10.plcp.part-*` | 186.9 GB | blueprint A (paper-style menu) |
| `buckets-200.tar` | 0.28 GB | the 200-bucket card abstraction for postflop streets. Required by every blueprint |
| `SHA256SUMS.models`, `SHA256SUMS.parts` | | checksums of the reassembled files and of every part |

Files are split into 25 GB parts for upload. Reassemble and check:

```bash
pip install -U huggingface_hub
hf download markokraemer/sharingan-1 --local-dir weights --include "sharingan-1.plcp.part-*" "buckets-200.tar" "SHA256SUMS.*"   # or everything
cd weights && sha256sum -c --ignore-missing SHA256SUMS.parts
cat sharingan-1.plcp.part-* > sharingan-1.plcp && sha256sum -c --ignore-missing SHA256SUMS.models && rm sharingan-1.plcp.part-*
mkdir -p ../data && tar xf buckets-200.tar -C ../data          # -> data/buckets-200/
```

The published full-file SHA256 for `sharingan-1.plcp`, used by the verified offline evaluations, is `2a4484f6fabf61470a67c941552f5a97c6f6474e14ea010ef94a41d7b5861300`. Verify the checksum manifest after reassembly. This model-card update changes no weights.

## Use it

From a clone of [the repo](https://github.com/markokraemer/poker-sharingan) (`RUSTFLAGS="-C target-cpu=native" cargo build --release`). The files are
memory-mapped: disk is required, RAM only makes it faster.

```bash
# inference server: POST /v1/act with a hand, get SHARINGAN-1's action (format: sharingan-1/serve/README.md)
./target/release/sharingan-serve --blueprint weights/sharingan-1.plcp --buckets data/buckets-200 --sizing medium --no-auth

# your own arena with C@2e10 as the house bot (match code revision and tracking options before comparing scores)
ARENA_BLUEPRINT=$PWD/weights/sharingan-1.plcp ARENA_BUCKETS=tables:$PWD/data/buckets-200 ARENA_SIZING=medium ./target/release/sharingan-arena --port 8080

# duplicate head-to-head of two blueprints, 6-max, 4M deals
./target/release/pluribus h2h --a weights/C-medium-4.77e10.plcp --sizing-a medium --b weights/sharingan-1.plcp --sizing-b medium \
    --buckets data/buckets-200 --players 6 --deals 4000000 --threads 32 --warm --out h2h.json

# gauntlet against the built-in bots; trained best response against a frozen blueprint
./target/release/bench gauntlet --hero compact:weights/sharingan-1.plcp --bucketer tables:data/buckets-200 --sizing medium --deals 20000
./target/release/pluribus exploit-compact --target weights/sharingan-1.plcp --sizing medium --buckets data/buckets-200 \
    --players 6 --iters 12000000 --threads 32 --mem-gb 120 --seat all --warm --eval-deals 20000 --seed 1 --out exploit.json
```

Each blueprint must be loaded with its own menu: `medium` for C and SHARINGAN-1, `local` for B, `paper` for A.

### Formats

* **PLCP1 (compact policy)**: the playable average strategy only. 8-bit action probabilities plus an infoset index, **about 12 bytes per infoset**.
  Produced by `pluribus compact`. Read by `bench`, `openpoker`, `search` (`--policy compact:...` / `search:...`).
* **PLBP4 (full checkpoint)**: `blueprint.bin` with the full regrets and average strategy, enough to **resume training** (`pluribus train --resume`).
  Companion `snapshots.sum` (PLSA1) holds the postflop average-strategy snapshots. Checkpoints are written tmp-then-rename.
* **Bucket tables**: `flop.u16`, `turn.u16`, `river.u16` map a suit-isomorphic hand index to a bucket id; `meta.txt` records `buckets 200 200 200`.
  Preflop is lossless (169 classes).

Historical training-store counts: the earlier C phase reached 12.2B infosets (78% of a 460-GB store); A reached 17.0B (store full). These are training-phase counts, not the size of every released checkpoint. The final SHARINGAN-2 compact contains about 14.5B infosets.

## Model details

* **Game:** 6-max (2-6 handled by the engine) no-limit hold'em, 100 bb stacks (blueprint). A separate 200 bb heads-up blueprint exists for Slumbot tests and is not part of this release.
* **Algorithm:** parallel external-sampling **Linear MCCFR** with negative-regret pruning. LCFR discount for the first 3.5% of the run, pruning from 1.7%
  (95% of iterations, threshold -300M, not on river or terminal actions), averaging from 6.9%, strategy interval 10k. The paper's minute-based schedule is scaled to iterations.
* **Abstraction:** preflop lossless; 200 buckets per postflop street (EMD k-means on equity histograms for flop/turn, OCHS for river). Action menus:
  `local` (B), `paper`-style (A), `medium` (C). **Menu must match at load time** (`--sizing`): evaluating a blueprint with the wrong menu gives garbage (about -2,364 mbb/game).
* **Real-time search (code, not weights):** depth-limited MC-LCFR with 4 continuation strategies at the leaves, Bayesian beliefs, off-tree re-search; final config searches turn and river only (`--set flop=0`), 200k iterations.
* **Action translation in the blueprint/player path:** the fixed source uses `BelowMenu::Smallest`, one-step synchronization checks, explicit handoff when synchronization is impossible, and a strength-based fallback. Known profitable attacks remain. This source behavior is separate from the model file and from any deployed worker revision.
* **Seeds:** 2019 for C and A.

## Training

| run | menu | iterations reached | wall | notes |
|---|---|---|---|---|
| B | `local` | 2e10 (finished) | ~9.5 h | |
| C, earlier phase | `medium` | 5.55e10 before resuming | ~58 h | 180k -> 355k it/s; subsequently continued to the full budget |
| A | `paper` | 3.43e10 (stopped) | ~66 h | store full at 17.0B infosets |
| C final (SHARINGAN-2) | `medium` | 1.83e11 (finished 2026-11-02) | | the paper's budget; beats SHARINGAN-1 by +5.1 ± 1.2, C@4.77e10 by +2.0 ± 1.0 (3 vs 3) |

Hardware: 64-core / 576 GB bare-metal servers, about EUR 1.86/h each. The runs B, C and A above add up to about 134 server-hours, about EUR 250 at that rate (training only).

## Evaluation

Unless noted otherwise: 6-max, duplicate deals + AIVAT, mbb/game (1 mbb/game = 0.1 bb/100), 95% CI. Full tables and caveats are in [docs/RESULTS.md](https://github.com/markokraemer/poker-sharingan/blob/main/docs/RESULTS.md).

| test | result |
|---|---|
| C@2e10 vs B-final (3+3, 4M deals/format) | +1.6 +- 1.3 |
| C@4.77e10 vs C@2e10 (3+3) | +3.8 +- 1.0 |
| A@2e10 vs C@2e10 (3+3) | -4.0 +- 1.3 |
| trained best response (12M iters) vs C@2e10 / C@4.77e10 / A@2e10 | -323 +- 62 / -290 +- 59 / -372 +- 61 (this limited attack loses; later adversarial-menu attacks below find profitable deviations) |
| original sombra strategy (tiny bets misread as checks) vs C@2e10 | fixed **offline** build **−420 ± 162**, 20,004 hands. These do not establish current live behavior; see the current reruns in docs/RESULTS.md |
| frozen adversarial-menu attacker (3M iters / 35 GB) vs fixed C@2e10 / final SHARINGAN-2, no search | **+2,307 +- 75 / +2,466 +- 75**, two deal seeds / 1.2M hands per target; verified model/learner inputs |
| API chip-rounding attacker vs C@2e10, search disabled | **+14,283 ± 910**, 20,004 hands at 500/1,000 blinds; actual guarded Core API, chance/attacker corrections only, API target decisions unexposed. This is not the native 50/100 blueprint protocol |
| strong profile LBR vs fixed C@2e10 (raw duplicate winnings, no AIVAT) | **+841 +- 162**, 20k deals / 120k hands; exactly reproduced on a verified public model copy |
| search (turn+river) vs 5 x C@2e10 | +87 +- 27 |
| search (turn+river) vs rule bots chart6 / loose / tight | +59 / +108 / +66 |
| Open Poker live, final config (with search), 2,329 hands | +77.6 bb/100, 95% CI [26, 129] (all-in adjusted) |
| offline vs SvanBot at its live budget, C@4.77e10 blueprint: 1 SvanBot vs 5 x blueprint / 1 blueprint vs 5 x SvanBot | SvanBot -46.9 +- 21.2 / blueprint +56.7 +- 34.7 |

## Intended use

Research on imperfect-information game solving, abstraction and real-time search; benchmarking other poker agents; studying
6-player strategy optimization; playing against the bot locally or on arenas that explicitly allow bots.

The sync fix is in source commit [05f59cc](https://github.com/markokraemer/poker-sharingan/commit/05f59cc) and later. Rebuild the Rust player/worker when adopting that fix; downloading the same weights does not update a previously compiled executable. This addresses the original translation bug, not all profitable attacks.

## Limitations and risks

* **Not evaluated against strong humans** or against other top solvers; external references are one bot arena and one offline bot, both with caveats.
* **Confirmed profitable deviations remain:** the strongest tested frozen trained learner wins +2,307 +- 75 against C@2e10 and +2,466 +- 75 against final SHARINGAN-2 without search. These finite attacks supply lower bounds, not equilibrium-gap estimates or guarantees about other attacks. Search-augmented play was not tested for exploitability.
* Trained for six-max, 100 bb stacks; native heads-up play is out of distribution. Very deep stacks need an external stack-reset guard (`--max-stack-bb`) or the strategy is out of distribution.
* Model weights and source-level fixes do not by themselves identify the version used by a hosted benchmark.
* The API has a separate real/scaled-chip replay and fallback path. A measured chip-rounding attack profits against C@2e10 with search disabled; the terminal-replay panic guard does not repair the poker weakness of check/fold fallback. See iteration 10 in the results.
* Off-menu bet sizes are handled by action translation; sizes far from the menu are the main exploitable surface in this class of agents.
* Self-play performance and the ability to exploit weak opponents are different measurements; do not infer either from the other.
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
