# Search buckets: per-flop, potential-aware, 500 per round

Paper (supplement): "The real-time search algorithm only uses lossless information abstraction
for the round it is in. For later rounds, it uses lossy information abstraction with 500
buckets per round ... determined separately for each flop using an algorithm that considers the
future potential of each poker hand. That algorithm combines potential-aware abstraction with
clustering based on earth-mover distance."

Code: `src/search_buckets.rs` (library), `examples/build_search_buckets.rs` (offline build),
`examples/search_bucket_quality.rs` (metrics). Search wiring: `SearchParams::search_buckets`
in `crates/search/src/solver.rs`.

## Design

For each of the 1,755 suit-isomorphic flops, clustering only within that flop:

* **River (500 buckets)**: every (hole, turn, river) situation of the flop (1,176 unordered
  turn/river pairs x 1,081 holes = 1.27 M). Features: exact OCHS (equity vs 8 opponent
  preflop-hand clusters) from `river_cluster_equities`, so card removal is exact. L2 k-means
  fitted on a 60k sample (20 iterations, k-means++ seeding), all 1.27 M situations then assigned
  to the nearest centroid. Bucket ids are sorted by strength. River bucket depends only on
  (hole, unordered {turn, river}), i.e. imperfect recall, as in the blueprint.
* **Turn (500 buckets)**: every (hole, turn) situation (49 x 1,128 = 55 k). Feature = the
  histogram over *this flop's river buckets* of the river card (46 equally likely rivers):
  potential-aware (Ganzfried & Sandholm 2014), so a flush draw (bimodal: misses or becomes the nuts)
  and a pair with the same equity (unimodal) get different histograms. Clustered with k-means
  under an earth mover's distance whose ground distance between two river buckets is the
  Euclidean distance between their centroids in OCHS space.
* **Flop**: no table (search is lossless on its own round; for a flop-root subgame the flop
  round uses exact hole cards and flop-level leaf infosets keep the blueprint bucket).

### Fast approximate EMD (deviation, documented)

Exact EMD between 500-bin histograms for 55k x 500 point/centroid pairs per iteration is far too
slow. We use an L1 embedding of a ground metric that approximates the centroid distance, so that
EMD = L1 between sparse vectors and k-means with mean centroids works (same trick the repo's
`Emd1d` metric uses for 1-D histograms):

1. **Tree ensemble (`BucketTree`)**: 4 three-level hierarchies over the 500 river centroids
   (k-means into ~64 groups then ~8; fanouts 8, 6, 11, 5 for diversity). Tree-EMD of two
   histograms is `sum_edges w_e |mass_e(p) - mass_e(q)|` (Indyk-Thaper); edge weights = half the
   mean sibling-centroid distance of that level, averaged over the trees.
2. **Exact 1-D strength EMD**: river buckets (ids sorted by strength) are binned into 64 bins;
   EMD along strength (ground distance = difference of bin mean OCHS strength) embeds as
   `gap_j * CDF_j`.
3. The two parts are each rescaled so their mean distance equals the mean centroid distance and
   averaged. Pearson correlation between this ground distance and the true centroid distance over
   all 500x499/2 bucket pairs: **~0.92-0.96** (mean 0.95 over the full build; tree-only with
   1 hierarchy was 0.48, 4 trees 0.61, + 1-D component 0.95). This mattered for quality: tree-only
   gave turn equity R^2 0.9935 (< blueprint 0.9980); with the 1-D component 0.9982.
4. Turn k-means: sparse L1 k-means (distance cost O(nnz) per point/centroid), k-means++ seed,
   fit on 20k sampled situations, 14 iterations, then all 55k assigned. Turn ids are ordered by mean
   river-bucket id of the centroid (strength proxy). Knobs (more trees, full fit sample, 25 iterations)
   moved R^2 by < 0.001; the ground-distance fidelity was the lever.

Remaining approximations: (1) EMD is the embedded approximation above, not exact transport;
(2) centroids are means (not medians) under L1; (3) fits use samples; (4) the opponent
clusters for OCHS come from `tables::opponent_clusters` (200k boards, cached in `opp.u8`).
(5) Within a flop we do not enforce isomorphism between hands that differ only by a stabiliser
of the flop; equal features give equal buckets up to centroid ties.

## Storage and lookup

* Canonical flops: `FlopCanon` (24 suit permutations, min sorted card tuple; 1,755 classes, tested).
  `canonicalize(flop) -> (id, perm)`; hole/turn/river cards are mapped with `perm` before lookup.
* `FlopBuckets` (one file `flop-XXXX.sb` per canonical flop, 3,654,472 bytes):
  `turn[hole_index * 52 + turn_card]` (1326 x 52 u16) and `river[hole_index * 1326 +
  hole_index(turn, river)]` (1326 x 1326 u16), `0xFFFF` = card collision. Plain direct-index
  tables, no per-lookup hashing; ~29% of the river table is unused (collisions) which we accept
  for O(1) lookups and zero indexing code (the isomorphic minimum would be ~2.4x smaller).
* **Full set built: `data/search-buckets-500/` on the dev box, 1,755 files, 6.0 GB** (+ `opp.u8`).
  Not pulled to the Mac. Fully loaded in RAM = 6.0 GB, trivially fits.
* `SearchBuckets::new(dir, BuildParams, cache_cap)`: LRU cache of flops (`OnceLock` per flop so a
  flop is loaded/built once even under concurrent solves). `for_flop([c0,c1,c2]) -> FlopView`
  loads the file if present, otherwise builds lazily and writes it. `FlopView::bucket(hole,
  board) -> Option<u32>` takes the board with the flop first (4 cards = turn bucket, 5 = river
  bucket, else `None`) and any suits. A search resolves the `FlopView` once per solve.

## Build time (dev box, 24 threads, box shared with a training agent)

| what | time |
|---|---|
| one flop, 24 threads, idle | **1.5 s** (river 0.7 s, turn 0.8 s) |
| one flop, 8 threads | 2.7 s |
| one flop, 1 thread | 16.9 s |
| all 1,755 flops, 24 threads (24 flops concurrent, loaded box) | **3,013 s (50 min)**, 1.72 s/flop amortised |

So lazy build is ~1.5 s per new flop with 24 idle threads: not within a 100 ms budget, hence
the precomputed set; loading a flop file is a single 3.6 MB read (a few ms). Lazy build remains a
correct fallback (used by the unit tests with `BuildParams::tiny()`).

## Quality vs the blueprint's 200 buckets (same flops, all situations, exact equities)

`search_bucket_quality`, 24 flops evenly spaced over the 1,755 ids (every situation of each flop,
exact river equity; turn equity = mean over the 46 rivers; potential = std of river equity over
the 46 rivers). Blueprint = `data/buckets-200` tables. Mean over flops:

| metric | search-500 (per flop) | blueprint-200 (global) |
|---|---|---|
| river R^2 of equity | **0.9976** (min 0.9965) | 0.9897 (min 0.9872) |
| turn R^2 of equity | 0.9975 (min 0.9950) | 0.9978 (min 0.9966) |
| turn R^2 of potential (std over rivers) | **0.9876** (min 0.9784) | 0.9763 (min 0.9534) |
| equal-equity (<0.01), different-draw (potential diff >0.06) pairs in same bucket (376k pairs, lower better) | **0.0000** | 0.0013 |
| control: equal-equity, equal-potential pairs in same bucket (higher = coarser) | 0.29 | 0.69 |
| buckets non-empty per flop (river / turn) | 500 / 500 | 143 / 159 of 200 |

Reading: river is clearly better (also uses 3.5x the buckets effectively); turn equity R^2 is
on par with the blueprint (0.9975 vs 0.9978, ~equal), potential R^2 and draw separation are better; the
blueprint almost never merges equal-equity/different-draw hands either (0.13% of such pairs), so
that check is a sanity test rather than a discriminator: the blueprint's 50-bin equity histograms
are already distribution-aware. The real difference is resolution: only 29% of equal-equity,
equal-potential pairs share a search bucket vs 69% in the blueprint, i.e. the abstraction is much
finer, since 500 buckets are spent within one flop. These are abstraction-quality numbers; whether
they translate into better play inside search is NOT yet measured (see below).

## Wiring into search

`SearchParams::search_buckets: Option<Arc<SearchBuckets>>` (default `None` = old behaviour).
When set and the subgame root is flop or later, the solver resolves the `FlopView` for the
root board's flop once and uses the per-flop buckets for (a) the keys of later-round (turn/river)
search infosets and (b) the keys of depth-limit leaf infosets on turn/river. **Blueprint
lookups always keep the blueprint's own buckets** (warm-start prior, the continuation-strategy
rollouts, belief updates): the blueprint was trained on those. A preflop-root subgame keeps the
blueprint buckets (no single flop to specialise on). `Solution::search_bucket_lookups` counts
lookups; test `per_flop_search_buckets_switch` (crates/search/tests/search.rs) checks default
off (0 lookups, field `None`), on (>0 lookups, one flop built, valid root strategies).

Usage:
```rust
let sb = Arc::new(SearchBuckets::new(Some("data/search-buckets-500".into()), BuildParams::default(), 64)?);
let params = SearchParams { search_buckets: Some(sb), ..Default::default() };
```

## Not done / next

* No head-to-head measurement of search with 500 per-flop buckets vs blueprint buckets (the old
  behaviour stays the default until that is measured: gauntlet/duplicate+AIVAT with the real
  blueprint once the paper-scale run exports).
* Search infosets get 500 instead of 200 buckets per street, so with the same iteration count
  each infoset sees ~2.5x fewer samples; the warm start from the blueprint mitigates this.
* Reproduce: `DEV_TARGET=flopabs scripts/dev.sh run 'cargo run --release -p abstraction --example
  build_search_buckets -- --out data/search-buckets-500 --threads 24'` (resumable) and
  `... --example search_bucket_quality -- --flops 24`.
