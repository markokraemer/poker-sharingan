# Poker Sharingan

An open-source Rust reproduction of the Pluribus method for six-player no-limit
Texas hold'em: training, blueprint inference, optional real-time search, and
reproducible duplicate-deal + AIVAT evaluation. Apache-2.0 licensed.

[SHARINGAN-2 weights](https://huggingface.co/markokraemer/sharingan-2) ·
[SHARINGAN-1 weights](https://huggingface.co/markokraemer/sharingan-1) ·
[Method](docs/METHOD.md) · [Evaluation](docs/EVALUATION.md)

## Models

SHARINGAN-2 is blueprint C trained for **1.83e11 iterations**. SHARINGAN-1 is the
**2e10-iteration checkpoint of the same run**. Both use the medium action menu,
200 postflop buckets, six players and 100 bb starting stacks. Real-time search
is optional Rust code, not part of the weight files.

The final blueprint scores **+5.09 ± 1.16 mbb/game** against SHARINGAN-1 in the
3-versus-3 format (4M duplicate deals, 95% CI). This measures relative playing
strength, not resistance to exploitation.

## Exploitation risk

**Known profitable attacks exist. Do not treat either model as unexploitable.**
Off-menu action translation and fallback decisions remain important weaknesses.
The original tiny-bet synchronization bug was fixed, but that did not remove all
profitable attacks. Experimental countermeasures that failed regression checks
were not promoted to the default policy.

A frozen 3M-iteration attacker nevertheless wins **+2,466 ± 75 mbb/game against the fixed final SHARINGAN-2 blueprint**, confirmed on two deal seeds with verified model/learner hashes (1.2M hands, duplicate + AIVAT). The same policy wins **+2,307 ± 75 against verified C@2e10**, with both seed results exactly reproduced on a fresh public model copy. An earlier frozen 1M attacker also wins +714 ± 67 against final SHARINGAN-2. Standard local best response also reproduces **+841 ± 162 against verified C**. The original host's supplied model file has inconsistent reads; new evaluations use a separately verified private copy. These are attack bounds against five blueprint copies, without search. Causes and fixes remain under investigation; these tests do not establish unexploitable play. [Measurements and input audit](docs/RESULTS.md#action-translation-leak-and-fix-nov-2026).

Work is paused at a documented checkpoint. No hosted service is required or
promised by this project. See [checkpoint status](docs/CHECKPOINT.md) for what was
kept, rejected and left incomplete.

## Use locally

Download and verify the model and bucket files using the
[model cards](model-cards/README.md), then build the Rust tools:

```bash
cargo build --release -p sharingan-serve -p bench -p exploit -p pluribus
./target/release/sharingan-serve \
  --blueprint weights/sharingan-2.plcp --buckets data/buckets-200 \
  --sizing medium --no-auth
```

Use `--no-auth` only in an isolated local environment. See the
[inference server documentation](sharingan-1/serve/README.md) for authentication,
request formats and search configuration. The weights are memory-mapped; plan
for sufficient disk space and substantial RAM for practical evaluation speed.

## Repository

- `engine/`: poker rules.
- `arena/`: evaluation harness, scoring, reference bots and self-hosted server.
- `sharingan-1/`: training, search and inference code for both model releases.
- `sdk/`: Python and TypeScript clients.
- `research/`: small-game experiments.
- `docs/`: methods, training instructions and measured results.

[Train from scratch](docs/TRAINING.md) · [Results](docs/RESULTS.md) ·
[Self-host](arena/server/README.md#self-host) · [API](docs/TABLE_API.md)

For research and local experimentation. Do not use bots to cheat on real-money
poker services.
