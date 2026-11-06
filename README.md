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

## Research limits

Both models have known exploitable weaknesses, particularly around off-menu bets.
They are research baselines, not guaranteed optimal strategies.
[Evaluation results](docs/RESULTS.md#action-translation-leak-and-fix-nov-2026) ·
[Research checkpoint](docs/CHECKPOINT.md)

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
