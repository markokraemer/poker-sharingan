# Evaluation pipeline: checkpoint -> compact policy -> h2h / exploiter / gauntlet

Everything here runs on the boxes, never on the Mac. Entry point for a whole evaluation:
`scripts/eval_checkpoint.sh A|B` (see "One-shot script").

```
 training box (niced, <=8 thr, <=100 GB)          dev box (<=16 thr)
 blueprint.bin (PLBP4) [+ snapshots.sum (PLSA1)]
        |  pluribus compact  (mmap, streaming)
        v
 compact.plcp (PLCP1, ~12 B/infoset)  --rsync, direct-->  data/eval/TAG/compact.plcp
                                                            |- bench gauntlet --hero compact:FILE
                                                            |- pluribus h2h --a FILE_A --b FILE_B
                                                            `- pluribus exploit-compact --target FILE
```

## 1. Compact format `PLCP1` (`sharingan-1/blueprint/src/compact.rs`)

The *final* policy of the blueprint, exactly the semantics of `snapshot::export_in_place`
followed by `Store::strategy` (what `pluribus export` + `BlueprintPolicy` play):

* first-round infosets (`with_avg`): normalised φ counters (current regret-matching strategy if
  all φ are zero);
* other infosets covered by the snapshot running sum (a snapshot taken after the infoset was
  allocated, sums not all zero): normalised snapshot sums;
* else the current regret-matching strategy (uniform if no positive regret).

Layout (little endian; each section starts on a 4096-byte boundary; the file is `mmap`ed):

| section | size | content |
|---|---|---|
| header | 4096 B | magic `PLCP1`, n, dir_bits D, blob_len, section offsets, source stats, file length |
| dir | (2^D + 1) x 8 B | `dir[b]` = index of the first key whose top D bits are >= b (D chosen for ~32 keys per bucket) |
| keys | n x 8 B | `infoset_key(history, bucket)` (the store's table key), strictly ascending |
| na | n x 1 B | number of actions of entry i |
| base | ceil(n/64) x 8 B | blob offset of entry 64*j |
| blob | sum(na) B | per entry `na` probabilities as u8 (p = v/255), each distribution sums to exactly 255 |

**Size: ~9.6 B per infoset (8 key + 1 na + 1/8 base + ~0.5 dir) + 1 B per action** = ~12 B per
infoset at ~2.5 actions per infoset. Paper-sizing run A (8.2 B infosets at 267 min):
~100 GB compact vs ~225 GB checkpoint (18 B record + 4 B/slot) and ~600 GB of RAM as a `Store`.

Lookup (`Compact::probs(history, bucket, &mut [f32]) -> bool`, lock-free, `&self`): directory
slot -> binary search of ~32 keys -> sum of <= 63 `na` bytes (one cache line) -> probabilities.
~0.4 us per lookup single-threaded on a warm 2 GB file (measured); 3-5 page faults when cold.

**Why u8 and not u16:** largest-remainder quantisation to 255 levels keeps every action
probability within 1/255 = 0.39 % of the exact one (typically <= 0.2 %); zero stays exactly zero.
u16 would add ~2.5 B per infoset (+20 %) to shrink an error that is far below the sampling noise of
any match we can run (a 0.2 % mis-weighted action is < 1 mbb/game). Measured worst error on
toy stores: 0.0034; on 2 M random samples of a 150 M-infoset synthetic checkpoint: 0.0031.

### Streaming build (`pluribus compact`)

```
pluribus compact --run DIR | --ckpt FILE [--sums FILE] --out FILE [--min-visits N]
                 [--threads 8] [--group-gslots 5] [--part-mb 512] [--tmp DIR] [--chunk 1048576]
```

The checkpoint (and the snapshot sums) are `mmap`ed and **never loaded**. A PLBP4 record
(key, arena offset, n, φ-flag) is in key order (hash-table order), but its probabilities live at
a random place of the arena (allocation order): a key-ordered pass would be one random page read
per infoset (tens of TB of IO at paper scale). So:

1. **Pass 1, arena order.** The arena is cut into groups of `--group-gslots` G slots (working set
   6 B/slot with sums: default 5 G slots = 30 GB of page cache). For each group the byte range
   of the arena and of the sums is prefetched *sequentially* (8 MB strips, all threads), then
   every record is scanned (sequential) and those whose offset lies in the group are
   computed (final-policy semantics above, quantised) and appended as `[key][n][n bytes]` to
   one of 2^pb temporary **key partitions** (top pb bits of the key, ~512 MB each, <= 512 files).
2. **Pass 2, key order.** Partitions are processed in parallel: read, sort by key, check for
   duplicate keys, and `pwrite` at their exact final position in each section (positions follow
   from the per-partition counts, so there is no sequential writer): keys, na, base, blob and the
   partition's slice of the directory. Header, fsync, rename; temp partitions deleted as they
   are consumed.

Properties: sequential IO only (records are re-read once per group, ~G_total passes); input
record order is irrelevant (no assumption on how `io::save` ordered them); output is
**byte-identical for any thread count / group size / partition size** (checked: 1 B infosets,
8 threads x 4 groups vs 16 threads x 1 group); anon memory ~ `threads x ~2.5 GB` in pass 2 and
a few hundred MB in pass 1, plus page cache (run it in a cgroup, as the script does:
`MemoryHigh=90G`, `MemoryMax=100G`). Temp disk ~ output size (next to `FILE`, `--tmp` to move).
Exact layout is sized after pass 1, so there are no sparse holes even with `--min-visits`.

* `--min-visits N` (default 0 = exact): drop first-round infosets with total φ < N and other
  infosets that have no positive regret and no snapshot coverage (never-trained, uniform).
  Dropped infosets fall back to check/call in the policy exactly like unknown ones.

Scale tests (`cargo run --release -p blueprint --example compact_scale -- N THREADS`; checkpoints
written by the real `io::save`, random keys, 3.6 actions/infoset incl. φ; dev box, file just
written so fully page-cached):

| infosets | checkpoint | threads / groups | time | compact | B/infoset | verified |
|---:|---:|---|---:|---:|---:|---|
| 150 M | 4.85 GB | 16 / 4 | 9 s (16 M/s) | 1.96 GB | 13.07 | 2 M samples, 0 bad, max err 0.0031 |
| 1,000 M | 32.35 GB | 16 / 4 | 43 s (23 M/s) | 12.89 GB | 12.89 | 2 M samples, 0 bad, max err 0.0031 |
| 1,000 M | 32.35 GB | 8 niced / 4 | 66 s (15 M/s), peak RSS incl. mapped cache 31.6 GB | identical bytes | | |

Extrapolation to run A (9.0 B infosets, 23.5 G slots, 40 GB sum file): ~10-15 min cached at 8
threads; cold, bounded by sequential disk reads: (records 160 GB + arena 94 GB + sums 47 GB) x
(1 + groups) -> roughly 15-40 min.

## 2. `CompactPolicy` (`arena/bench/src/compact.rs`)

```rust
let p = CompactPolicy::open(Path::new("compact.plcp"), bucketer /* Arc<dyn Bucketer+Send> */, sizing /* BetSizing */)?;
let f: Arc<dyn PolicyFactory> = Arc::new(p);
```

It *is* a `BlueprintPolicy` (`BlueprintPolicy::from_source(name, Arc<dyn bp::Source>, ..)`):
same abstract-history tracking, deterministic pseudo-harmonic translation (AIVAT-exposable),
and the same check/call fallback for unknown infosets or action-count mismatches
(`miss_rate()`). The only change in `bench` is the small `bp::Source` trait (default behaviour
and all existing APIs unchanged; `Shared.source = None` takes the old path). `bench gauntlet
--hero compact:FILE --bucketer tables:DIR --sizing S` works too. The exploiter's frozen target
is `exploit::cli::CompactTarget`.

## 3. `pluribus h2h`

```
pluribus h2h --a FILE_A --sizing-a S --b FILE_B --sizing-b S [--players 6] [--deals N]
             [--threads N] [--seed 1] [--buckets data/buckets-200|coarse] [--buckets-a D] [--buckets-b D]
             [--warm] [--out result.json]
```

Three duplicate formats on identical deals, AIVAT-scored (both sides expose exact probabilities):
`3A+3B` (block seatings AAABBB rotated through all 6 seats + the two alternating patterns),
`1A vs 5B` and `1B vs 5A` (hero rotates through every seat). Prints a table and a JSON
(`results[i].mbb.{mbb,se,ci95}`, `raw_mbb`, `var_ratio_deal`, per-seat). mbb/game, positive =
the first-named side wins. A and B may use different sizings (each policy tracks its own
abstract history) but must share the card abstraction (or pass `--buckets-a/-b`).

Reading it: A better than B => `3A+3B` > 0, `1A vs 5B` > 0 and `1B vs 5A` < 0. Both 1-vs-5 rows
positive means each side beats copies of the other (non-transitivity / the bots' abstractions
exploiting each other), both negative the opposite. Self-match is exactly 0 +- 0 under common
random numbers (test).

## 4. `pluribus exploit-compact`

`pluribus exploit-compact --target FILE --sizing S --buckets DIR --players 6 --iters N --threads 16
--mem-gb 60 --seat all --eval-deals N [--fine L] [--extract average|argmax] [--lbr-deals N]
[--warm] [--out result.json]` -- the same trained best response as `exploit` (one learner seat
vs 5 frozen copies, all seats, MCCFR learner store capped at `--mem-gb`, duplicate + AIVAT
evaluation vs `CompactPolicy`), but the target is the mmapped PLCP1 file (RSS ~0, no store).
`exploit --compact` is the same code (`exploit::cli`); the `exploit` crate no longer depends on
`pluribus` (it used only `sizing`/`TableBucketer`, now taken from `bench::bp`), which is what
allows `pluribus` to call it. JSON: `exploitation.avg_mbb.{mbb,ci95}`, per-seat, learner stats,
`target_store`. Interpretation as in `arena/exploit/RESULTS.md` (profile best response, lower
bound on exploitability in the abstract game).

## 5. One-shot script

```
scripts/eval_checkpoint.sh A|B [--deals 2000] [--exploit-iters 20000000] [--exploit-mem-gb 60]
                           [--min-visits N] [--vs FILE:SIZING] [--skip-gauntlet|--skip-h2h|--skip-exploit|--skip-eval]
                           [--run-dir ABS_DIR] [--tag TAG --no-compact] [--keep 2]
```

1. rsyncs the sources to `~/pluribus-eval` on the box (a separate tree: `remote.sh sync` would
   `--delete` the training dir) and builds there with its own target dir (systemd unit, Nice=19,
   idle IO class, 4 cores) -- the running `pluribus-train` binary/service is never touched;
2. hard-links `blueprint.bin` (+ `snapshots.sum`) so the inode survives the next checkpoint's
   rename, runs `pluribus compact` as a detached unit (`Nice=19`, `IOSchedulingClass=idle`,
   `CPUQuota=800%`, `MemoryHigh=90G`, `MemoryMax=100G`, no swap; survives ssh drops), unlinks;
3. copies the compact file box -> dev **directly** (`rsync -e ssh` from the box with the Mac's
   ssh-agent *forwarded*: the key never lands on a box; ~117 MB/s measured A -> dev, i.e. ~15 min
   for 100 GB); falls back to `ssh box cat | ssh dev 'cat > f'` (RAM only, not the Mac's disk);
   verifies the size; maintains `data/eval/latest-{A,B}.{plcp,sizing,tag}` on dev; keeps the
   newest 2 per box on dev and 1 on the box;
4. on dev (`RAYON_NUM_THREADS=16`): `bench gauntlet`, `pluribus h2h` (A = latest-A vs B = latest-B
   whenever both exist), `pluribus exploit-compact`; logs in `logs/eval-TAG.log`, results in
   `data/eval/TAG/{compact.json,gauntlet.txt,h2h-A-vs-B.json,exploit.json}`.

Plumbing was exercised end-to-end on box B with a synthetic 2.4 M-infoset checkpoint (compact 4 s,
transfer 2 s, all three evaluations ran).

## 6. Tests (dev box, `--release`)

| test | result |
|---|---|
| `blueprint::compact::tests::compact_matches_export_semantics` | 30 k-infoset store, two snapshots (late infosets, perturbed regrets, all-negative/all-zero/untouched cases, φ with zero counts), checkpoint both in random order and key-ordered with local shuffles, chunk/window (4096,2048), (777,300), (16384,65536); every infoset vs `export_in_place` + `Store::strategy`: max error 0.0034 <= 1/255; also no-snapshot semantics (φ / current strategy), `--min-visits` filter; a too-small window is reported as an error |
| `blueprint::compact::tests::quantize_sums_to_255_and_is_close` | 2000 random distributions: sum 255, error <= 1/255, zeros stay zero |
| `bench/tests/compact.rs` (HU and 6-max) | trained toy blueprint, snapshots + export: `CompactPolicy` vs `BlueprintPolicy` over the exported store, 400 random hands each, 1.2 k / 3.2 k decisions: max probability difference 0.0027, identical action sets, identical miss rate |
| `pluribus/tests/eval_pipeline.rs::h2h_self_match_is_zero` | 6-max, 3000 deals x 3 formats: 0.0 +- 0.0 |
| `...::h2h_detects_a_clear_difference` | 60 k-iteration vs 200-iteration toy blueprint: `3A+3B` +2974 +- 250, `1A vs 5B` +4431 +- 470, `1B vs 5A` -5989 +- 539 mbb/game |
| `...::exploit_compact_agrees_with_exploit_on_store` | HU toy: `exploit` on the store 6976 +- 598 vs `exploit-compact` 7041 +- 600 mbb/game |
| existing `bench`, `blueprint`, `exploit` suites | unchanged, all pass |

## 7. Real numbers (2026-10-13)

**Full-scale correctness.** B's checkpoint (1,953,658,056 infosets, 5.0 G slots) + snapshot sums:
`pluribus compact` (new algorithm, 8 threads) 104 s -> 22.94 GB, 11.74 B/infoset; then
`examples/compact_verify` loads the checkpoint into a `Store`, runs `export_in_place` with the same
sums and compares `Store::strategy` of **every** infoset: 0 missing, 0 bad, worst error 0.00314
(bound 1/255 = 0.0039). Output size is identical to the earlier-algorithm file used for the h2h.

| | A (paper sizing, run-paper) | B (local sizing) |
|---|---|---|
| checkpoint | it 1.5275 B, 276 GB + 41 GB sums | it 4.50 B, 55 GB + 10 GB sums |
| infosets | 9,594,614,275 (1.66 B phi, 7.7 B snapshot-covered) | 1,953,658,056 |
| compact | 113.96 GB, 11.88 B/infoset | 22.94 GB, 11.74 B/infoset |
| compaction (8 thr, nice 19, idle IO) | 975 s (6 arena groups: 707 s pass 1, 268 s pass 2) | 305 s |
| box -> dev (rsync, forwarded agent) | 1406 s (81 MB/s) | 194 s (118 MB/s) |

Gauntlet, 6-max, AIVAT mbb/game (2000 deals): see `data/eval/*/gauntlet.txt` on dev. A: tag +175,
lag -45, nit +84, station +1869, maniac -975; B: tag +366, lag +1041, nit +98, station +3147,
maniac +1549. (Heads-up rows are meaningless for 6-max blueprints: histories miss -> check/call.)

**A vs B head to head**, 2,000,000 deals/format, AIVAT, positive = A wins (4 threads, ~4 min per format):
3A+3B **+11.7 +- 2.3**; 1A vs 5B +3.6 +- 3.9; 1B vs 5A **-31.2 +- 3.8** (B alone loses to five A).
A (paper menu, 1.5 B it) is slightly ahead head to head although B has 3x the iterations and beats the
built-ins more; B's coarser menu likely pays translation loss against A's extra bet sizes.

**Exploiter vs A** (`exploit-compact`, 6-max, all seats, 16/12 threads, 60 GB learner):
4 M iterations (28 min): -322 +- 229 (undertrained); **12 M iterations (91 min, 2.2 k it/s): +470 +- 127
mbb/game** (20 k deals, 597 M learner infosets, 1.0 % miss). Still rising with iterations, so a lower bound.
Cost is the traversal (54 % `traverse`, 15 % target lookup), not the file.
