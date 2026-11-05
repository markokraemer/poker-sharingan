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
- name: SHARINGAN-2
  results: []
---

# SHARINGAN-2: the full-budget Poker Sharingan blueprint

SHARINGAN-2 is blueprint **C@1.83e11** of [Poker Sharingan](https://sharingan.poker),
an open reproduction of the Pluribus method (Brown & Sandholm, *Science* 2019) for
six-player no-limit Texas hold'em at 100 bb. Training finished on November 2, 2026.
[SHARINGAN-1](https://huggingface.co/markokraemer/sharingan-1) is the **2e10-iteration
checkpoint of the same run**; SHARINGAN-2 is the final 1.83e11-iteration release.

This repository contains strategy-table weights and card-abstraction tables.
Training, inference, search and evaluation are implemented in the
[Rust workspace](https://github.com/markokraemer/poker-sharingan). `library: custom`
means the Rust crates `blueprint` and `bench` read the PLCP1 files; there is no
Transformers or PyTorch loader. Real-time turn/river search is optional code,
not part of the weight file. The checkpoint comparisons below use **no search**.

## Public arena and model names

The public **`sharingan-1` track uses five copies of C@2e10 (SHARINGAN-1), without
search**. The browser play page uses that checkpoint too. The public `/v1/act`
configuration uses C@2e10 with turn/river search. Download and self-host the final
file below to use the full SHARINGAN-2 blueprint. The track ID remains
`sharingan-1`; it is not the model's training budget or a promise about the deployed
Rust revision.

On November 5, the exact original sombra submission scored **+4,761 ± 492 mbb/game**
in a fresh verified hosted run against that public **C@2e10** track (20,004 hands,
zero forfeits). This is not a result against final SHARINGAN-2 or the fixed offline
build. [Hosted run record](https://arena.sharingan.poker/v1/benchmarks/sharingan-1/runs/w_da35215b0163).

Model weights, action-translation code and deployed worker versions are separate.
A local fixed-build result must not be presented as a live hosted result. Record
the model checksum, sizing, buckets, code revision and tracking options with every
evaluation; the arena's blueprint fingerprint alone does not establish all of them.

## Results

Scores are **mbb/game**, with 95% confidence intervals: 1,000 mbb/game is one big
blind per hand, or 1 mbb/game = 0.1 bb/100. Unless noted otherwise, games are six-max
at 100 bb with duplicate deals and AIVAT. Sample sizes differ between experiments.

### Checkpoint strength

These blueprint-versus-blueprint runs use **4 million duplicate deals per format**.
The last column reports the other side's score.

| test | 3 + 3 seats | 1 SHARINGAN-2 vs 5 | 1 other checkpoint vs 5 SHARINGAN-2 |
|---|---:|---:|---:|
| SHARINGAN-2 vs SHARINGAN-1 (C@2e10) | **+5.09 ± 1.16** | +8.60 ± 1.89 | −7.64 ± 1.88 |
| SHARINGAN-2 vs C@4.77e10 | **+1.97 ± 0.97** | +3.76 ± 1.57 | −1.95 ± 1.58 |

### Adversarial evaluation

The final blueprint has **confirmed profitable deviations**. The strongest tested
frozen trained attacker wins **+2,466 ± 75 mbb/game against final SHARINGAN-2**,
without search. This combines two evaluation seeds, 100,000 duplicate deals per
seat/seed, **1.2 million hands**, with verified model and learner files. The same
3M-iteration / 35-GB adversarial-menu learner wins **+2,307 ± 75** against verified
C@2e10. These are finite-attack lower bounds, not equilibrium-gap estimates.

| experiment | result for the attacker | scope |
|---|---:|---|
| frozen adversarial-menu learner, 3M iterations | **+2,466 ± 75** | final SHARINGAN-2; two seeds / 1.2M hands |
| earlier frozen adversarial-menu learner, 1M iterations | **+714 ± 67** | final SHARINGAN-2; two seeds / 1.2M hands |
| original sombra strategy after the translation fix, fresh rerun November 5 | **−527 ± 154** | fixed offline final blueprint; 20,004 hands, zero forfeits |
| historical 12M-iteration learner | −247 ± 58 | limited earlier learner; its loss does not establish robustness |

The original tiny-bet/check translation bug is fixed in the Rust source. Stronger
attacks still profit. Experimental policy corrections that have not completed all
regression and normal-strength gates are not part of the retained baseline.
Search-augmented exploitability has not been established. Follow the
[current results and input-integrity audit](https://github.com/markokraemer/poker-sharingan/blob/main/docs/RESULTS.md#action-translation-leak-and-fix-nov-2026)
for exact commands, confidence intervals, reruns and deployment distinctions.

The earlier fixed offline sombra reference, −390 ± 156, reproduced exactly before
the fresh rerun. Both model files and all original submission files were verified.
The fresh hosted C@2e10 score above and these offline final-model scores use
different deal seeds and execution paths; do not pool them or infer a deployment
source version from the difference. The rerun changes no strategy weights or code.

Historical six-max gauntlet scores for SHARINGAN-2 against nit / TAG / LAG / maniac /
calling station / random are +125 ± 35 / +303 ± 56 / +933 ± 85 / +1,622 ± 99 /
+2,572 ± 118 / +4,004 ± 197. These are separate experiments from the 4M-deal
checkpoint comparisons, and do not establish resistance to adaptive attacks.

## Files and download

| file | size | contents |
|---|---:|---|
| `sharingan-2.plcp.part-*` | 169.9 GB reassembled | final C@1.83e11 strategy, PLCP1 compact, about 14.5B infosets, `medium` menu |
| `buckets-200.tar` | 0.28 GB | required 200-bucket postflop card abstraction, shared with SHARINGAN-1 |
| `SHA256SUMS.models`, `SHA256SUMS.parts` | small text files | full-file and part checksums |

The weights are split into 25-GB parts. Download, reassemble and verify:

```bash
pip install -U huggingface_hub
hf download markokraemer/sharingan-2 --local-dir weights
cd weights
sha256sum -c SHA256SUMS.parts
cat sharingan-2.plcp.part-* > sharingan-2.plcp
sha256sum -c SHA256SUMS.models
mkdir -p ../data
tar xf buckets-200.tar -C ../data
```

The reassembled `sharingan-2.plcp` SHA256 used in the verified evaluations is
`0f57b29605cb0fc5f2ecf4c1fcb266df6321d8459784b1577a30d54bb94c5b8f`.
Check against the repository's checksum manifest. This card update changes no weights.

## Use it

Build the [code repository](https://github.com/markokraemer/poker-sharingan):

```bash
RUSTFLAGS="-C target-cpu=native" cargo build --release

# Local inference using the final blueprint and optional turn/river search.
# --no-auth is for a local server; see sharingan-1/serve/README.md for access control.
./target/release/sharingan-serve --blueprint weights/sharingan-2.plcp \
    --buckets data/buckets-200 --sizing medium --no-auth

# A self-hosted arena using the final blueprint, without search.
ARENA_BLUEPRINT=$PWD/weights/sharingan-2.plcp \
ARENA_BUCKETS=tables:$PWD/data/buckets-200 ARENA_SIZING=medium \
    ./target/release/sharingan-arena --port 8080

# Reproduce the three-format checkpoint comparison; SHARINGAN-1 must also be downloaded.
./target/release/pluribus h2h --a weights/sharingan-2.plcp --sizing-a medium \
    --b weights/sharingan-1.plcp --sizing-b medium --buckets data/buckets-200 \
    --players 6 --deals 4000000 --threads 32 --seed 1 --out h2h.json
```

Always use the **`medium` sizing menu** with this file. The strategy is memory-mapped;
sufficient disk is required, while RAM/page cache determine speed. PLCP1 is the
playable average strategy, not the full regrets/checkpoint required to resume training.

## Training details

- Game: six-player no-limit hold'em, 100 bb starting stacks.
- Run: C, seed 2019, resumed from checkpoints to 1.83e11 iterations.
- Algorithm: parallel external-sampling Linear MCCFR with negative-regret pruning;
  the paper's time-based schedule is scaled to iterations.
- Abstraction: lossless preflop classes, 200 buckets on each postflop street, `medium` action menu.
- Training hardware: 64-core / 576-GB bare-metal servers.
- Turn/river search, action translation and fallback policies are implemented in Rust.
  They can change independently of the published strategy file.

The sync fix is in source commit [05f59cc](https://github.com/markokraemer/poker-sharingan/commit/05f59cc) and later. Rebuild the Rust player/worker when adopting that fix; downloading the same weights does not update a previously compiled executable. This addresses the original translation bug, not all profitable attacks.

## Limitations and intended use

This is a research and benchmarking release, not the original Pluribus system.
It has not been evaluated against strong humans, and no superhuman or equilibrium
claim is made. Known attacks profit against its blueprint. Finite attacks do not
provide an upper bound on exploitability, and six-player robustness cannot be
proved by the tests reported here.

The blueprint is trained for six-max, 100 bb; heads-up play is out of distribution
and loses to simple built-in bots in the reported tests. Unusual stack sizes,
off-menu bets, translation handoffs and fallback decisions remain important failure
surfaces. Self-play strength and exploitation of weak opponents are different metrics.

Use on research/local arenas that allow bots. Real-money poker sites prohibit bots;
do not use this release to cheat on them.

## Citation

```bibtex
@article{brown2019superhuman,
  title   = {Superhuman AI for multiplayer poker},
  author  = {Brown, Noam and Sandholm, Tuomas},
  journal = {Science}, volume = {365}, number = {6456}, pages = {885--890}, year = {2019}
}
```
