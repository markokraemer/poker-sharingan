# data/

Large data is **not** stored in git (see `.gitignore`).

## data/buckets-200/ (required to run any blueprint)

The 200-bucket card abstraction: `flop.u16` (~2.5 MB), `turn.u16` (~27 MB), `river.u16` (~235 MB), `meta.txt` (`buckets 200 200 200`); about 264 MB in total.
Download `buckets-200.tar` from the Hugging Face repository `TODO/sharingan` and extract it here, or rebuild it:

```bash
cargo run --release -p abstraction --bin build_buckets
```

## data/pokerbench/ (optional, for `arena/referee`)

The PokerBench CSVs (~88 MB) are used by the `referee` crate to measure per-spot accuracy. PokerBench is published by its authors on Hugging Face
(`RZ412/PokerBench`, Apache-2.0 according to its dataset card). It is not redistributed in this repository; download the four `*_game_scenario_information.csv` files from there.

## data/run-*/ and data/probe-*/

Training runs and probe outputs are written here and are git-ignored.
