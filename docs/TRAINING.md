# Training and resuming

How the blueprints in this repository were trained, and how to reproduce or continue a run. Everything below assumes a Linux machine; the
numbers come from 64-core / 576 GB servers. **Do not train on a laptop**: a paper-scale blueprint needs 400-500 GB of RAM.

## Build

```bash
RUSTFLAGS="-C target-cpu=native" cargo build --release -p pluribus
```

You also need the card abstraction tables in `data/buckets-200/` (download from Hugging Face, see [`data/README.md`](../data/README.md), or rebuild with
`cargo run --release -p abstraction --bin build_buckets -- data/buckets-200 200 200 200 200000`, about 30 s on 128 cores).

## Action menus (`--sizing`)

* `medium` (variant C): our best menu, 8 preflop opens. Used for the final bot.
* `paper` (variant A): our reconstruction of the paper's menu (up to 14 preflop raise sizes). The paper did not publish the exact menu.
* `local` (variant B): a smaller menu that fits in less memory.
* `tiny`: test-size game.

The sizing used to train must be the sizing used to evaluate or play a checkpoint; a mismatch silently gives garbage.

## Start a run

```bash
./target/release/pluribus train --out data/run-medium --iters 183000000000 --sizing medium --players 6 --mem-gb 460 \
    --checkpoint-min 360 --seed 2019 --eval-min 60 --eval-deals 2000 --eval-threads 8
```

* `--iters` sets the total run length. The paper's minute-based schedule is mapped to iterations proportionally (`scaled_schedule(iters)` in `sharingan-1/pluribus/src/main.rs`):
  LCFR discount `(t/10)/(t/10+1)` for the first 3.5%, pruning after 1.7% in 95% of iterations (threshold -300M, never on the river or terminal actions), averaging after 6.9%, strategy interval 10k.
* `--mem-gb` caps the lock-free infoset store. Size it to about 85% of RAM. When it is full, training continues gracefully without allocating new infosets.
* Checkpoints (`blueprint.bin`, PLBP4) and `snapshots.sum` (PLSA1) are written every `--checkpoint-min` minutes; saves are tmp+rename.
* `--eval-*` runs periodic AIVAT-scored matches against the built-in bots and writes `eval.jsonl`.
* Run it under a supervisor (e.g. systemd with `Restart=on-failure`); `scripts/remote.sh start ARGS` does that on a remote box and adds `--resume`.

## Resume

Pass `--resume` (same `--out`, same sizing, same seed). The run directory must contain `blueprint.bin`, `snapshots.sum`, `progress.txt` and `config.txt`.

```bash
./target/release/pluribus train --out data/run-medium --iters 183000000000 --sizing medium --players 6 --mem-gb 460 \
    --checkpoint-min 360 --seed 2019 --eval-min 60 --eval-deals 2000 --eval-threads 8 --resume
```

Our best line (variant C) was stopped at 5.55e10 iterations at about 355k iterations/s and was still improving; reaching 1.83e11 iterations takes about
130 more hours on one 64-core box and corresponds to roughly 8 days of total compute (the paper's budget). That run is in progress and becomes SHARINGAN-2.

Changing `--iters` on resume rescales the schedule. That is harmless once every phase (LCFR, pruning, averaging) has passed.
To keep a checkpoint, hard-link it (`ln -f blueprint.bin blueprint-<it>.bin`, same for `snapshots.sum`): saves are tmp+rename. Each snapshot fold is committed together with a
checkpoint, and a resume trims `snapshots.sum` to the checkpoint, so a crash never leaves the averages out of step with the regrets.

## Compact export

```bash
./target/release/pluribus compact --ckpt data/run-medium/blueprint.bin --sums data/run-medium/snapshots.sum \
    --out C.plcp --min-visits 0 --threads 64
```

PLCP1 stores 8-bit probabilities plus an infoset index (about 12 B/infoset). `--min-visits N` drops rarely visited infosets for a smaller file.

## Learnings from training (all fixed unless noted)

**Algorithms and engine**

* **Action translation** is where a 6-max blueprint leaks. The naive translation let a paper-menu exploiter win +26k mbb/g
  against B. A "shadow game" (track the abstract state separately from the real one) cut it to +700. With the `medium`
  menu it is essentially closed (+3 ± 98).
* **UPDATE-STRATEGY** must skip σ=0 opponent lines and must not allocate. Fixing that gave a 2.3–4× speedup.
* More perf fixes:
  * a contended atomic node counter → thread-local counters;
  * THP madvise;
  * 8-byte history keys;
  * a lock-free per-infoset index;
  * a 618 MB/s checkpoint writer.
* Throughput **rises** late in training (C 180k → 355k it/s, A 144k → 220k) as fewer new infosets get allocated.
* **Search vs blueprint**: iterations were the lever (20k → 200k: −42 → +87; 500k no better).
  * Opponent beliefs must use the blueprint's likelihood: they *are* the blueprint.
  * A parallel solve was 10× slower than single-thread (shared atomics plus a mutex) until it got per-thread counters and its own pool.
  * The exact HU vector solver doesn't help in 6-max.
* **Flop search leaks EV against off-model opponents** (rule bots): −120 mbb/g vs the blueprint. Turn/river-only search recovers most of it and keeps the full gain vs blueprint-like play.
  * Belief "tremble" didn't help (ε=0.05 no change, 0.2 worse).
  * The trade-off: flop search buys robustness against odd bet sizes (probe −10.9k vs −7.5k).
* **Exploiter caveat**: the ABR is bounded by its learner's abstraction and 12M iterations. "Unexploitable" means
  unexploitable *by this exploiter*.

**Memory, checkpoints and training ops**

* The checkpoint loader once sized the index at `max(n × 5/4)`. A 16B-infoset checkpoint then needed 457 GB of index
  (vs 320 GB in training) and was **OOM-killed on resume**. Fix: `n × 7/8`. Peak RSS on resume is now ≈ training RSS.
* Store-full is graceful: training continues without allocating new infosets. Size `--mem-gb` to ~85% of RAM.
* Saves are tmp+rename, so `ln -f blueprint.bin blueprint-<it>.bin` preserves checkpoints at no cost. `snapshots.sum` is
  updated in place, so it can't be hard-link-preserved.
* Changing `--iters` on resume rescales the schedule. That's harmless once every phase (LCFR, prune, avg) has passed.

## Operational gotchas



* Never `pkill -f`/`pgrep -f` a pattern that appears in your own `ssh ... "cmd"` line: it matches and kills the session.
  Kill by pid or parent pid.
* **Never edit a shell script while it is running.** Bash reads it incrementally, so the running instance breaks. Copy to a new file.
* `remote.sh sync` uses `rsync --delete`. It now excludes `*.log/*.out/*.stop/*.jsonl`, but before that it could wipe a box's `train.log`.
* `ssh host 'cd x && cmd &'` forks a subshell that keeps ssh open (hang). Use `cd x || exit; cmd &`.
* macOS has no `setsid`. Use `nohup … & disown` (plus `caffeinate -i` to stop idle sleep).
