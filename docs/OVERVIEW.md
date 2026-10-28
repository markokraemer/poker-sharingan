# Poker Sharingan: overview

<!-- badges: TODO replace once the repo is public -->
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](../LICENSE)
[![Weights on Hugging Face](https://img.shields.io/badge/weights-Hugging%20Face%20(TODO)-yellow.svg)](https://huggingface.co/TODO/sharingan)
[![CI](https://img.shields.io/badge/CI-TODO-lightgrey.svg)](#)

**SHARINGAN** is an agent-built, from-scratch, open reproduction of **Pluribus** (Brown & Sandholm, *Science*, 2019),
the superhuman 6-player no-limit Texas hold'em AI. It is a single Rust workspace that covers the whole stack:

* a **blueprint** trained with parallel external-sampling **Linear MCCFR** with negative-regret pruning,
* a **200-bucket** card abstraction per postflop street (169 lossless preflop classes),
* **depth-limited real-time search** (nested Monte Carlo LCFR with **4 continuation strategies**, Bayesian beliefs, off-tree re-search),
* action abstraction plus **shadow-game action translation**,
* an evaluation stack: duplicate matches, **AIVAT**, trained best response (exploitability proxy), LBR, rule bots, RLCard, Slumbot, and a live Open Poker client.

The CLI and the main crate are still called `pluribus`, after the paper this project reproduces. Crate and binary names were
deliberately not renamed; "SHARINGAN" is the name of the project.

The code, the abstraction recipe and the results below are what a mostly autonomous coding-agent workflow produced under human direction, on
rented bare-metal servers (about EUR 480 of compute so far, see [Compute](#compute)). Everything reported here was measured; the
limits of each measurement are stated next to it.

> **Status.** A final training run to the paper's 8-day compute budget is in progress.
> TODO(final numbers): update the tables marked `TODO(final)` below and the Hugging Face links once it finishes.

## Results

All 6-max numbers are **duplicate-deal + AIVAT** variance-reduced, in **mbb/game** (thousandths of a big blind per hand; 10 mbb/game = 1 bb/100),
with 95% confidence intervals. Positive means the first-named bot wins. Blueprints: **A** = paper-style action menu, **B** = a smaller
`local` menu, **C** = `medium` menu (the best of the three, and the one used in the final bot). Names like `C@4.77e10` mean "C after 4.77e10 MCCFR iterations".

### Head-to-head between our own blueprints (4M deals per format)

| A vs B | 3A + 3B | 1A vs 5B | 1B vs 5A (B's side) |
|---|---|---|---|
| C@2e10 vs B-final | +1.6 +- 1.3 | +1.1 +- 2.2 | -1.7 +- 2.2 |
| C@4.77e10 vs B-final | +3.4 +- 1.3 | +3.8 +- 2.2 | -5.3 +- 2.2 |
| **C@4.77e10 vs C@2e10** | **+3.8 +- 1.0** | +5.3 +- 1.6 | -6.8 +- 1.6 |
| A@2e10 vs B-final | -1.8 +- 1.4 | -5.8 +- 2.4 | -0.7 +- 2.3 |
| A@2e10 vs C@2e10 | -4.0 +- 1.3 | -5.7 +- 2.2 | +5.8 +- 2.2 (C) |

Ranking at 2e10 iterations: C > B > A. More training clearly helps C (and it was still improving when stopped at 5.55e10).
TODO(final): add the row `C@final vs C@4.77e10` once the full-budget run is evaluated.

### Exploitability (trained best response, 12M iterations; negative = the exploiter loses)

| target | exploiter with the same action menu | exploiter with the paper menu |
|---|---|---|
| C@2e10 | -323 +- 62 | +3 +- 98 (at 1.03e10 iters) |
| C@4.77e10 | -290 +- 59 | not run |
| A@2e10 | -372 +- 61 | not run |

Caveat: the exploiter is bounded by its own abstraction and 12M iterations. "Not exploitable" here means *not exploitable by this exploiter*,
not a proven bound. The real-time-search bot has **not** been measured for exploitability.

### Real-time search

Hero (search) in every seat vs 5 copies of the field; blueprint alone = 0 by definition against itself.

| field | search, flop on | **search, turn+river only (`flop=0`, final)** | blueprint alone |
|---|---|---|---|
| 5 x C@2e10 blueprint (30k deals) | +78 +- 31 | **+87 +- 27** | 0 |
| 5 x B-final blueprint | +101 +- 22 / +110 +- 31 | n/a | 0 |
| chart6 rule bot (7k deals) | +16 | +59 | +97 |
| chart6-loose | +32 | +108 | +124 |
| chart6-tight | -3 | +66 | +125 |
| RLCard DQN | +1012 | +709 | +476 |

Search is a clear win against the blueprint it is built on, but it *loses* EV against weak fixed rule bots compared with playing the blueprint
alone (flop search costs about 120 mbb/game there), which is why the final config searches only the turn and river.
Search iterations were the main lever (20k: -42, 200k: +87, 500k: no better). Details and the full hill-climb log: [`docs/RESULTS.md`](RESULTS.md) and [`crates/search/RESULTS.md`](../crates/search/RESULTS.md).

### External bots and live play

* B-final vs 5 x chart6 / chart6-loose / chart6-tight / RLCard DQN: **+134 / +174 / +130 / +587** mbb/game. Every external bot loses when seated with five copies of B.
* Heads-up 200 bb blueprint vs **Slumbot** (2,400 live hands): +14 +- 66 bb/100, not significant.
* **Open Poker** (openpoker.ai, a public bot arena with other developers' bots), all-in-luck-adjusted bb/100:

| session | config | hands | result |
|---|---|---|---|
| 1 | C@1.03e10 blueprint | 1,347 | -29 +- 139 |
| 2 | C@2e10 + search on all streets | 578 | +95 +- 122 |
| 3 | turn/river search, before a deep-stack fix | 104 | -445 (one 1,105 bb jam) |
| **4 (final config)** | **C@2e10 + `flop=0` search + `--max-stack-bb 150`** | **2,329** | **+77.6 bb/100, 95% CI [26, 129]** |

Session 4 had 25 stack resets, 0 search fallbacks and one reconnect give-up. A few hundred live hands are pure noise (+-150 bb/100); this is a single
bot on a single arena against whatever opponents were online, not a proof of superhuman strength. Session 3 is shown because it motivated the
stack-reset logic (cash-game stacks accumulate on that site, while the bot was trained for 100 bb).

### Benchmark vs SvanBot (offline, with caveats)

[SvanBot](https://github.com/SvanLabs/SvanBot) (reportedly #1 on the Open Poker ladder) was run through its real live decision policy with its learning
opponent model, replaying every public history through its own engine (0 replay mismatches or panics over 25M decisions). Duplicate + AIVAT, 95% CI, mbb/game:

| arm | SvanBot budget | deals | result |
|---|---|---|---|
| 1 SvanBot vs 5 x our blueprint (C@4.77e10) | simulation budget (2.5k samples) | 400k | SvanBot -66.1 +- 7.4 |
| 1 our blueprint vs 5 x SvanBot | simulation budget | 400k | **us +42.4 +- 7.7** |
| 1 SvanBot vs 5 x our blueprint | **live budget (1.6M samples, ~0.39 s/decision)** | 50k | **SvanBot -46.9 +- 21.2** |
| 1 our blueprint vs 5 x SvanBot | **live budget** | 20k | **us +56.7 +- 34.7** |
| 1 our final bot (search) vs 5 x SvanBot | simulation budget | 40k | +18.3 +- 24.7 (not significant) |

Caveats, in order of importance:
1. SvanBot's production fleet also uses a learned neural response model and parameters tuned on its own hand database. We ran a fresh install's defaults, with empty per-opponent models.
2. This is a "who is closer to equilibrium" test. SvanBot is a much stronger *exploiter of weak fixed bots* than our blueprint (it beats rule bots roughly twice as hard), which is exactly what a ladder full of weak bots rewards.
3. The search arms used the small simulation budget and are not statistically significant.
4. The SvanBot bridge (`scripts/extbots/svan/extbridge.rs`) is included; SvanBot itself (Apache-2.0) is not vendored.

### Compute

| item | amount |
|---|---|
| Hardware | 3 x 64-core / 576 GB bare-metal training boxes + 1 x 32-core / 251 GB eval box |
| Blueprint C | 5.55e10 iterations, ~58 h, 12.2B infosets, 180k -> 355k iterations/s (throughput rises late in training) |
| Blueprint A (paper menu) | 3.43e10 iterations, ~66 h, 17.0B infosets (store full at ~500 GB) |
| Blueprint B (local menu) | 2e10 iterations, ~9.5 h |
| **Spend so far** | **about EUR 480** of compute (~282 box-hours), plus a few euros per month for storage |
| **Final run to the paper's 8-day budget** | TODO(final): iterations reached, wall time, infosets, EUR, and the new evaluation numbers |

The paper's minute-based schedule was mapped to iterations proportionally (`scaled_schedule` in `crates/pluribus/src/main.rs`):
LCFR discount for the first 3.5% of the run, pruning from 1.7% (95% of iterations, threshold -300M, never on the river or terminal actions), averaging from 6.9%.

## Coming soon: public play-against API and open benchmark

We plan to host a **public API to play against SHARINGAN** and an **open, duplicate-deal benchmark** so that other 6-max agents can be measured on the
same footing (variance-reduced, with confidence intervals). The protocol draft is [`docs/TABLE_API.md`](TABLE_API.md). Nothing is live yet; this section
will be replaced with usage instructions when it is.

## Quickstart

Requirements: a recent stable Rust toolchain. Training needs about 400-500 GB of RAM; playing and evaluating a blueprint is far cheaper but the
compact weight files are large (tens to ~190 GB), so check the sizes in [`MODEL_CARD.md`](../MODEL_CARD.md) first.

```bash
git clone https://github.com/markokraemer/poker-sharingan && cd poker-sharingan
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

### 1. Get the weights and the card abstraction

Weights and abstraction tables are **not** in git. Download them from Hugging Face (TODO: replace with the real repository once published):

```bash
pip install -U huggingface_hub
huggingface-cli download TODO/sharingan --local-dir ./weights      # https://huggingface.co/TODO/sharingan
```

The 200-bucket abstraction tables (`flop.u16`, `turn.u16`, `river.u16`, `meta.txt`, ~264 MB) must end up in `data/buckets-200/`.
They can also be rebuilt from scratch with `cargo run --release -p abstraction --bin build_buckets` (compute-heavy, see `crates/abstraction`).

### 2. Evaluate a blueprint

```bash
# duplicate head-to-head of two compact blueprints, 6-max, 4M deals
./target/release/pluribus h2h --a weights/C.plcp --sizing-a medium --b weights/B.plcp --sizing-b medium \
    --buckets data/buckets-200 --players 6 --deals 4000000 --threads 32 --warm --out h2h.json

# gauntlet against the built-in bots
./target/release/bench gauntlet --hero compact:weights/C.plcp --bucketer tables:data/buckets-200 --sizing medium --deals 20000

# trained-best-response exploiter against a frozen compact blueprint
./target/release/pluribus exploit-compact --target weights/C.plcp --sizing medium --buckets data/buckets-200 \
    --players 6 --iters 12000000 --threads 32 --mem-gb 120 --seat all --warm --eval-deals 20000 --seed 1 --out exploit.json
```

Search experiments (blueprint + real-time search vs a field) use `crates/search/examples/vsbp.rs`:

```bash
cargo run --release -p search --example vsbp -- --compact weights/C.plcp --sizing medium --buckets data/buckets-200 \
    --hero search --field B --set flop=0 --deals 20000 --seed 1
```

`./target/release/bench` prints its own usage (`gauntlet|match|matrix|table|lbr|slumbot|mkbp`).

### 3. Play against it / live bot

The Open Poker client (`crates/openpoker`, see its README) runs the final configuration (blueprint + turn/river search + stack reset):

```bash
export OPENPOKER_API_KEY=...        # never commit this
./target/release/openpoker live --policy search:weights/C.plcp --sizing medium --buckets data/buckets-200 \
    --search-threads 16 --search-budget-ms 5000 --set flop=0 --hands 1000 --buy-in 2000 --no-auto-rebuy --max-stack-bb 150
```

Note that `openpoker live --help` starts a real session; run `openpoker` with no arguments for usage. The release profile uses `panic=abort`, so
wrap long sessions in a restart loop (`scripts/openpoker-live.sh` is an example). For your own table server, `docs/TABLE_API.md` is the API
contract we drafted for a 6-max duplicate benchmark.

### 4. Train

```bash
./target/release/pluribus train --out data/run-medium --iters 183000000000 --sizing medium --players 6 --mem-gb 460 \
    --checkpoint-min 360 --seed 2019 --eval-min 60 --eval-deals 2000 --eval-threads 8 --resume
./target/release/pluribus compact --ckpt data/run-medium/blueprint.bin --sums data/run-medium/snapshots.sum --out C.plcp --min-visits 0 --threads 64
```

`train` resumes from `blueprint.bin` (PLBP4 checkpoint) when present. Size `--mem-gb` to about 85% of RAM; when the infoset store is full, training
continues without allocating new infosets.

## Crate map

| crate | role |
|---|---|
| `cards` | card primitives, 7-card evaluator, RNG |
| `iso` | suit-isomorphism indexer (port of Waugh's hand-isomorphism, see NOTICE) |
| `nlhe` | 2-6 player NLHE engine, action abstraction, shadow-game action translation |
| `abstraction` | equity features, EMD k-means, 200-bucket blueprint tables, per-flop 500-bucket search tables |
| `blueprint` | MCCFR trainer, lock-free infoset store, checkpoints (PLBP4), snapshots, compact policy (PLCP1) |
| `search` | real-time search (`SearchPolicy`), MC-LCFR + vector CFR+, beliefs; hill-climb log in `RESULTS.md` |
| `bench` | Policy API, built-in/rule/RLCard bots, duplicate matches, AIVAT, LBR, Slumbot client, probes |
| `exploit` | trained best response (ABR) against a frozen blueprint |
| `referee` | PokerBench accuracy, exact heads-up river CFR+ referee |
| `openpoker` | Open Poker live client: protocol, engine replay and cross-check, action mapping, search guard, stack reset, trace analyzer |
| `pluribus` | CLI: `train`, `export`, `compact`, `h2h`, `exploit-compact` |
| `cfr-lab`, `search-lab` | small-game verification labs (Kuhn/Leduc, exact exploitability) |

Repository layout:

```
crates/            Rust workspace (all crates above)
docs/              METHOD.md, TRAINING.md, EVALUATION.md, RESULTS.md, DEVLOG.md, TABLE_API.md, BENCHMARK_LANDSCAPE.md
data/              git-ignored; buckets-200 tables go here (see data/README.md)
scripts/           portable helpers for running on rented boxes (hosts via PLURIBUS_HOST / PLURIBUS_DEV_HOST), extbots/, experiments/
scripts/infra/     examples tied to one specific cloud setup, scrubbed of ids (see its README)
MODEL_CARD.md      Hugging Face model card for the weights
```

Documentation: [METHOD](METHOD.md) | [TRAINING](TRAINING.md) | [EVALUATION](EVALUATION.md) | [RESULTS](RESULTS.md) | [DEVLOG](DEVLOG.md) | [TABLE_API](TABLE_API.md) | [Benchmark landscape](BENCHMARK_LANDSCAPE.md).

## Deviations from the paper

* **Action menu:** the final bot uses our `medium` menu, not our reconstruction of the paper's menu (the paper does not fully specify it). C beat A by about 4 mbb/game at equal iterations.
* **Search only on turn and river** (`flop=0`), not the flop: flop search leaked EV against off-model opponents.
* **Opponent beliefs in search use the blueprint likelihood** (the opponents *are* the blueprint in our tests); our own seat uses the search average.
* **Search budget:** 200k Monte Carlo iterations per decision on 16-32 threads (about 0.24-1 s per searched decision), not the paper's time-based budget.
* **Schedule:** the paper's minute-based schedule is scaled to iterations proportionally.
* **Training budget:** the paper used about 8 days on a 64-core server with 512 GB RAM; our runs are multi-box and separate per variant. TODO(final): state exactly how the final run compares.
* **Abstraction:** 200 buckets per street from our own EMD k-means (flop/turn) and OCHS (river) features; the paper's exact abstraction is not public. A 500-bucket per-flop search abstraction was built but is off in the final config because it did not help.
* **Opponent modelling for the shadow game:** action translation tracks an abstract state separately from the real one; the naive version was exploitable by +26,000 mbb/game and the shadow game cut that to about +700 (and about zero with the `medium` menu).
* The paper evaluated against top human professionals; we have no such evaluation.

## Limitations

* Not evaluated against strong humans or other top solvers. Open Poker and SvanBot are the only external 6-max references, both limited as described above.
* Exploitability numbers are lower bounds from one exploiter; the search bot has not been tested for exploitability at all.
* The blueprint is trained for 100 bb stacks; the live bot needs the stack-reset guard on sites where stacks accumulate.
* Large weights (tens to hundreds of GB) and high RAM requirements for training.
* Statistical resolution: +-1-2 mbb/game needs millions of duplicate deals; small search runs (7k deals) are +-70.
* Heads-up play uses a separate 200 bb blueprint (only lightly evaluated against Slumbot).

## Responsible use

This is research code, and a strong poker agent is dual-use. Online poker sites ban bots, and using this software to play on real-money sites
violates their terms and may be illegal in your jurisdiction. Use it for research, for studying game-theoretic play, for benchmarking, and to play
*against* locally or on arenas that explicitly allow bots (such as Open Poker). Do not use it to play against people who have not agreed to play a bot.

## Third-party code and data

See [`NOTICE`](../NOTICE). In short: `crates/iso` is a port of Kevin Waugh's hand-isomorphism indexer; PokerBench and the RLCard/SvanBot benchmarks are used as external
evaluation data/opponents but are not redistributed here. The Pluribus paper text is not included; get it from *Science* (doi:10.1126/science.aay2400).

## Citation

This project reproduces:

```bibtex
@article{brown2019superhuman,
  title   = {Superhuman AI for multiplayer poker},
  author  = {Brown, Noam and Sandholm, Tuomas},
  journal = {Science},
  volume  = {365},
  number  = {6456},
  pages   = {885--890},
  year    = {2019},
  doi     = {10.1126/science.aay2400}
}
```

If you use SHARINGAN, please cite the repository (TODO: add a `CITATION.cff` with the release tag; repository: https://github.com/markokraemer/poker-sharingan).

## License

Apache-2.0, see [`LICENSE`](../LICENSE).
