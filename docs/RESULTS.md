# SHARINGAN results

Everything measured so far, in one place. Conventions: 6-max no-limit hold'em, 100 bb, duplicate deals + AIVAT variance reduction,
mbb/game = thousandths of a big blind per hand (1 mbb/game = 0.1 bb/100), 95% confidence intervals. Blueprints: **A** = paper-style menu,
**B** = `local` menu, **C** = `medium` menu (final). See [EVALUATION.md](EVALUATION.md) for how each number is produced.

SHARINGAN-1 = C@2e10 + turn/river search. The inference API (`/v1/act`) plays it with search; the leaderboard's sharingan-1 track, `sharingan play --vs sharingan-1`
and the browser play page use the released C@2e10 blueprint without search. The search results below (+87 vs the blueprint, Open Poker live) are the search bot.
The full-budget run (C@final, 1.83e11 iterations) is training and will be SHARINGAN-2.


### Training runs

| run | sizing | machine | iters reached | wall | infosets | it/s | notes |
|---|---|---|---|---|---|---|---|
| B | `local` | B | **2e10** (done) | ~9.5 h after the perf fixes | – | – | finished first |
| C | `medium` | A | 2e10 in 30.6 h → 4e10 → **5.55e10** (stopped) | ~58 h | 12.2B (78% cap at 460 GB) | 180k → 355k | the extended run was still improving |
| A | `paper` | A→D | 2e10 → **3.43e10** (stopped) | ~66 h | **17.0B (store full)** | 105k → 220k | memory-capped at 500 GB |

Seeds: 2019 for C and A. Checkpoints every 360 min.

### Head-to-head (4M deals/format unless noted)

| A vs B | 3A+3B | 1A vs 5B | 1B vs 5A (B's side) |
|---|---|---|---|
| C@2e10 vs B-final | +1.6 ± 1.3 | +1.1 ± 2.2 | −1.7 ± 2.2 |
| C@4.77e10 vs B-final | **+3.4 ± 1.3** | +3.8 ± 2.2 | −5.3 ± 2.2 |
| **C@4.77e10 vs C@2e10** | **+3.8 ± 1.0** | +5.3 ± 1.6 | −6.8 ± 1.6 |
| A@2e10 vs B-final | −1.8 ± 1.4 | −5.8 ± 2.4 | −0.7 ± 2.3 |
| A@2e10 vs C@2e10 | −4.0 ± 1.3 | −5.7 ± 2.2 | +5.8 ± 2.2 (C) |

Ranking at 2e10: C > B > A. Longer training clearly helps C.

### Exploitability (trained best response, 12M iters, 1 learner vs 5 copies; negative = exploiter loses)

| target | own-menu learner | paper-menu learner |
|---|---|---|
| B-final (before the translation fix) | – | +26,007 |
| B-final (after the shadow translation fix) | −205 | +700 ± 149 |
| C@2e10 | −323 ± 62 | +3 ± 98 (at 1.03e10) |
| C@4.77e10 | −290 ± 59 | – |
| A@2e10 | −372 ± 61 | – |

### Real-time search (see the search hill-climb below)

| hero vs 5 × field | default (flop on) | **flop=0 (final)** | blueprint alone |
|---|---|---|---|
| 5 × C@2e10 blueprint (30k) | +78 ± 31 | **+87 ± 27** | 0 |
| 5 × B-final blueprint | +101 ± 22 / +110 ± 31 | – | 0 |
| chart6 (7k, same deals) | +16 | +59 | +97 |
| chart6-loose | +32 | +108 | +124 |
| chart6-tight | −3 | +66 | +125 |
| RLCard DQN (retrained on B) | +1012 | +709 | +476 |
| off-menu probe bet=1.5 (lower = better for us) | −10.9k | −7.5k | −7.7k |

Defaults: 200k iterations; opponents' beliefs from the blueprint likelihood, our own seat's from the search average;
16–32 thread pool; mean ~0.24–1 s per searched decision.

### External bots and live play

* B-final vs 5× chart6 / loose / tight / RLCard: +134 / +174 / +130 / +587. Every external bot loses when seated with 5× B.
* HU 200 bb blueprint vs **Slumbot** (2,400 live hands): +14 ± 66 bb/100. Not significant.
* **Open Poker** (openpoker.ai), all-in-adjusted bb/100:

| session | config | hands | result |
|---|---|---|---|
| C10B | C@1.03e10 blueprint | 1,347 | −29 ± 139 |
| Csearch | C@2e10 + full search | 578 | +95 ± 122 |
| CsearchTR | turn/river search, **before the deep-stack fix** | 104 | −445 (one 1,105 bb AK jam) |
| **CsearchTR2** | **final: C@2e10 + flop=0 search + max-stack-bb 150** | **2,329** | **+77.6 [26, 129]** (CI excludes 0) |

The TR2 session had 25 stack resets, 0 fallbacks and 1 reconnect give-up (the restart loop recovered in 30 s).

### vs SvanBot (2026-10-15/16)

The setup:
* SvanBot commit `9a46bc6`. Bridge `extbridge` (in the tar) speaks our `ext:` JSON-lines protocol.
* It calls SvanBot's real live policy, `sv10_policy::policy::decide_with`, with its learning `ModelStore`.
  It sees every finished hand with all hole cards, so it adapts during the match.
* The public history is replayed through SvanBot's own engine (`Situation::from_hand`).
* Over 25M decisions there were 0 replay mismatches, errors or panics. SvanBot played VPIP 18–27% / PFR 13–18%.
* Our side is the C@4.77e10 blueprint (`B`) or the final bot (`search --set flop=0`).
* Units are mbb/game, AIVAT, 95% CI.

| arm | SvanBot budget | deals | result |
|---|---|---|---|
| 1 SvanBot vs 5 × our blueprint | sim (2.5k samples, 0.75 ms/decision) | 400k | SvanBot −66.1 ± 7.4 |
| 1 our blueprint vs 5 × SvanBot | sim | 400k | **us +42.4 ± 7.7** |
| 1 SvanBot vs 5 × our blueprint | **live (1.6M samples, ~0.39 s/decision)** | 50k | **SvanBot −46.9 ± 21.2** |
| 1 our blueprint vs 5 × SvanBot | **live** | 20k | **us +56.7 ± 34.7** |
| 1 our final bot (search) vs 5 × SvanBot | sim | 40k | us +18.3 ± 24.7 |
| 1 SvanBot vs 5 × our final bot | sim | 12k | SvanBot −30.3 ± 43.6 |
| cold models (`SVAN_LEARN=0`): SvanBot vs 5 × blueprint / blueprint vs 5 × SvanBot | sim | 100k | −101.2 ± 16.9 / +144.5 ± 19.0 |

SvanBot vs 5 × rule bots (sim budget, 20k deals):

| opponents | SvanBot | our blueprint |
|---|---|---|
| chart6 | +278 | +139 |
| chart6-loose | +233 | +179 |
| chart6-tight | +502 | +124 |
| nit / tag / lag | +726 / +764 / +1655 | |
| station / random | +14,643 / +4,268 | |

Reading:
* Our near-equilibrium bot beats SvanBot head-to-head in both seatings, also at SvanBot's full live budget.
* SvanBot's online adaptation helps it, by about 35–100 mbb/g compared with its cold priors, but not enough.
* SvanBot is a far stronger *exploiter of weak fixed bots*, which is what an Open Poker ladder rewards.

Caveats:
* SvanBot's live fleet also has a learned neural response model and learner-promoted params from its own hand database.
  We ran a fresh install's defaults. Its learned per-opponent models start empty offline.
* With 110 parallel workers, each SvanBot instance saw only a slice of the hands to learn from.
* Search arms used the sim budget and small samples, so they are not significant.

---

---

## Search hill-climb: search: 6-max hill-climb - why search lost to B, and the fix (real final blueprint B, 2e10 it, local sizing)

Setup: `examples/vsbp` (`--set k=v,...` overrides, see `apply_sets`), box B (128 threads, one search per thread inside the match,
`parallel=false`), 6-max, hero in every seat vs 5 x B, duplicate cards + AIVAT, 95% CI, mbb/game, positive = search ahead of B.
"20k deals" arms share seed 5 (same cards in every arm); confirmation runs use fresh seeds 7 / 11. `bpbel` = `blueprint_beliefs`.
Old default (20k MC iterations, beliefs from the search average): -150 +- 50 on seed 5 (-118 +- 41 on the earlier 30k-deal run).

## Diagnosis
1. **Not a bug, not the translation: the MC solve is under-converged at 20k iterations and that costs ~100-150 mbb/game.**
   Iteration scaling with everything else fixed (bpbel + own-seat search belief, all streets, default menus): 20k -42 +- 34 (40k deals),
   100k +58 +- 29 (40k), 200k +87 +- 27 (40k), 500k +58 +- 35 (20k). It saturates at +60..+100 from ~100k iterations.
   Without the belief change: 5k -75, 20k -150, 80k(bpbel) +7, 200k +25 (20k deals each, +-40..60).
2. **Beliefs: opponents are the blueprint.** Bayes updates of the OPPONENTS' ranges with the search's under-converged average
   strategy distort them (-150 -> -52 at 20k, same cards); updating them with the blueprint likelihood (what they actually play) removes
   it. Our own seat keeps the search average (`own_search_beliefs`, our real strategy): 20k -65 -> -9 (paired, 20k deals). Defaults changed.
3. **Where it was lost (20k its, legacy beliefs, 20k deals):** river-only -22 +- 32, turn-only +27 +- 35, turn+river -19 +- 40, flop-only -51 +- 43, all streets -150.
   Flop split by live players: HU flops -42 +- 36, 3-way -9 +- 22, 4+ -0.4 +- 6 (so multiway flops are NOT the problem: HU flops solved by MC are). The sum of
   small per-street losses (noisy solves + belief damage on later streets) is the -150.
4. **Things that did NOT matter (20k its, 20k deals; all within noise of each other):** cont4 vs blueprint leaves (-153 vs -150), warm start 0 / 32 (-162 / -88 legacy),
   focus_own 0 (-328, clearly worse; 0.8 -95), average vs final iterate (final -101 legacy / -38 with bpbel, avg stays default), sizing small vs default menu (-5 vs -52),
   search menu = blueprint "local" menu (-45), opponent-pooled root infosets by blueprint bucket (new `opp_pool`: -65, no gain).
5. **Vector (exact HU) solver in 6-max does not help** (legacy beliefs, 20k its): HU flop pooled 300 passes -101 +- 55 (1000 passes -78 +- 51),
   turn pooled -97 +- 52, turn exact infosets 1000 passes -98 +- 50, river exact -137 +- 51; all at or below MC with the same beliefs. An equilibrium solve does not
   exploit B the way the warm-started, hand-focused MC solve does, and MC is the only solver that handles the multiway nodes anyway. Left opt-in (`vector=false`).
   Implementing multiway vector LCFR was therefore NOT pursued: the measured lever is convergence (iterations), which is cheap in wall clock (below).
6. **Speed bug found and fixed (matters for real latency, not for these numbers):** a parallel solve was 10x SLOWER than single-threaded: a shared `nodes`/`leaves`
   atomic per node, a single `Mutex` per traversal for diagnostic root values and a `fetch_max` per node serialised all threads. Counters are now per traversal,
   root values per-thread stripes. Also a dedicated pool is driven from a plain OS thread (blocking a rayon worker in `install` let it steal other searches).
   Measured: 200k iterations, 32 threads (`--par-threads 32`, deals sequential): see table.

## Changes (SearchParams defaults)
`iters` 20_000 -> **200_000**; `blueprint_beliefs` false -> **true** (opponents updated with the blueprint); new `own_search_beliefs` = **true**
(own seat with the search average); new `offtree_search_beliefs` = **true** (opponent actions that were off the search tree and added as extras are still
scored with the search average: the blueprint has no likelihood for them; this keeps part of the off-menu robustness). New knobs (off by default): `opp_pool`, `max_players`, `min_flop_players`, `pool`. `vsbp --set` for ablations.
Everything else unchanged (warm start 8, focus_own 0.5, average strategy, cont4 leaves, MC for all streets, vector off).

## Arms (hero search vs 5 x B)
| arm | deals/seed | AIVAT mbb/game | mean search ms (1 thread) |
|---|---|---|---|
| old default: 20k, search-average beliefs | 20k / 5 | -150.3 +- 49.5 | 429 |
| 5k | 20k / 5 | -74.8 +- 57.8 | 137 |
| river only / turn only / turn+river / flop only (20k, legacy beliefs) | 20k / 5 | -21.9 / +27.2 / -19.2 / -51.3 (+- 32..43) | 114 / 276 / 209 / 640 |
| 20k, bpbel | 20k / 5 | -51.6 +- 48.8 | 424 |
| 20k, bpbel + own-seat search belief (new belief rule) | 40k / 7 | -42.4 +- 34.4 | 426 |
| same, no flop search | 40k / 7 | -21.2 +- 28.3 | 209 |
| bpbel 80k | 20k / 5 | +6.9 +- 41.6 | 2171 |
| bpbel 200k | 20k / 5 | +24.9 +- 38.2 | 3738 |
| bpbel + own, 100k | 40k / 7 | +57.5 +- 28.9 | 1851 |
| bpbel + own, 200k | 40k / 7 | +86.7 +- 26.6 | 3722 |
| bpbel + own, 500k | 20k / 7 | +58.2 +- 35.4 | 9629 |
| 200k, bpbel + own (no off-tree rule) | 60k / 11 (fresh seed) | +100.9 +- 22.0 | 3756 |
| **NEW DEFAULT (200k, bpbel + own + off-tree search belief)** | **30k / 11** | **+95.3 +- 31.1** | 3745 |

Latency, default config, a search on a 32-thread dedicated pool (`--par-threads 32`, one deal at a time, 300 deals, 893 searches): **mean 237 ms, max 528 ms**
(1 thread: mean 3.7 s, max 40 s; ~8x speedup, hogwild contention on the shared root infosets limits it). Budget headroom to ~2M iterations exists but 500k was not better.

## Off-menu probe robustness with the new default (probe `bet=1.5` vs 5 x search, 1500 deals seed 11, hero = probe, lower = better for search)
Reference: B -7604 +- 200 (60k deals), old search default (20k, search-average beliefs) -16344 +- 1377 (this run) / -15364 +- 691 (6000 deals, earlier).
| search config | probe mbb/game |
|---|---|
| old default (20k, search-avg beliefs) | -16344 +- 1377 |
| 20k, blueprint opponent beliefs | -10800 +- 1345 |
| 20k, + off-tree search belief | -13635 +- 1389 |
| 200k, legacy beliefs | -12240 +- 1267 |
| 200k, blueprint opponent beliefs (no off-tree rule) | -9327 +- 1209 |
| **200k, + off-tree search belief (NEW DEFAULT)** | **-10944 +- 1262** |
Reading: the robustness gain over B (-7.6k) is still there (probe loses ~44% more vs search than vs B; it was 2x), but the belief change that makes search stronger vs B
(+100 mbb/game) gives back part of it: scoring the probe's off-menu bet with the blueprint's translated likelihood is a worse model of a probe than the search
average. The off-tree rule recovers part of it at no cost in strength (+95 vs +101). Probe CIs are +-1300, so rows within ~2k are not distinguishable. Not run:
probe bet=0.1 / 3 with the new default (only 1.5 was asked for).

(1 thread = sequential solve inside the 128-thread match; max 40 s. The default config needs ~13 cpu-seconds per hand.)

# search: shadow-game blueprint history (off-menu translation port) - real final blueprint B

Change: `search::bp::Blueprint` no longer builds a blueprint `History` from real-state action lists. Every blueprint read in search
takes a `nlhe::abstraction::Shadow` (the bench policies' shadow game; no logic duplicated) plus the real state: action list, history key
and node come from the shadow; abstract choices map back to real amounts via `Shadow::real_action`. Covered readers: Bayes belief
updates over earlier rounds (`SearchPlayer::update_event`, per-event shadow snapshot), blueprint play (`Blueprint::dist`), warm-start
priors (`Blueprint::prior`, search actions mapped onto the shadow node's list with `Shadow::translate`), continuation rollouts
(`Solver::rollout` steps a shadow), subgame-tree history (`Solver::child` observes on a per-node shadow), subgame root
(`Subgame.bp_hist` is now a `Shadow`, `round_root()` returns one), preflop-search "far off-tree" trigger (compares against the shadow
node's raise sizes as real chip amounts). Within the current round's subgame off-tree actions are still added to the tree (unchanged).
If the shadow desyncs (e.g. a bet below the smallest size treated as a check), lookups degrade as in `BlueprintPolicy` (check/call,
no prior, no Bayes update). `Blueprint::from_source` / `with_opts` added: Blueprint can sit on a compact file (`bench::bp::Source`),
and `TrackOpts::LEGACY` reproduces the old tracking for A/B. New example `examples/vsbp.rs` (compact B, any hero/field of
`search|B|B-legacy|probe:...`, 6-max duplicate + AIVAT).

Tests: `cargo test --release -p search` all green (8 unit + 14 + 2 integration). New `off_menu_preflop_raise_keeps_blueprint_lookups_in_tree`:
opponent makes an off-menu preflop raise; afterwards the SearchPlayer's blueprint distributions (preflop reply and flop) equal
`BlueprintPolicy`'s exactly (so same miss behaviour; it folds >5%, i.e. not the legacy calling station), the raiser's belief row is
non-uniform (Bayes hit), and flop warm-start priors hit for every sampled hole.

## Numbers (final B = 2e10 it, sizing local, buckets-200 compact; 6-max, hero in every seat vs 5 copies, duplicate + AIVAT, 95% CI)
Search = `SearchParams::default()` (20k MC iterations, warm start 8, focus_own 0.5, average strategy, preflop search on) with
`parallel=false` inside matches. Search cost: mean 258 ms per search (~2.5 searches/hand-with-search), max ~1.9 s vs B field; 0.47-0.67 s
mean when the field is 5 searchers facing probe bets (many re-solves).

Probe's mbb/game (lower = better for the agent; station baseline -6704):
| probe | B shadow (60k deals) | B legacy (60k) | SearchPolicy shadow | SearchPolicy legacy tracking |
|---|---|---|---|---|
| bet=0.1 | -7465 +- 119 | -1033 +- 77 | -17383 +- 581 (6000 deals) | -16980 +- 1007 (2000) |
| bet=1.5 | -7604 +- 200 | -16171 +- 216 | -15364 +- 691 (6000) | -17046 +- 1187 (2000) |
| bet=3 | -7443 +- 198 | -15026 +- 258 | -17085 +- 685 (6000) | -19381 +- 1194 (2000) |

Search vs 5x B (search hero, mbb/game, AIVAT): shadow **-118 +- 41** (30,000 deals = 180k hands; 5000-deal pilot -157 +- 102);
legacy tracking -93 +- 71 (10,000 deals). Shadow vs legacy difference is within noise.
B's lookup miss rate in these matches: 0.01% on-menu; search's own lookups hit the same nodes.

Reading: (1) search is far harder to exploit by off-menu probes than the blueprint (-15k..-17k vs -7.4k; it folds/raises
rather than station-calling, partly by re-solving with the probe's size added); (2) the legacy-vs-shadow search difference is
small here (CIs overlap everywhere): the probes bet postflop, where the bug mostly shows through the *earlier-round* history, and
the dominant protection is the in-subgame extras; bench-level gains from the fix were on blueprint play. The port is still
correct/necessary for consistency (priors, beliefs, rollouts, preflop lookups after off-menu raises); (3) probes' CI is +-600..1200,
not +-150: probe variance is ~100x that of a normal opponent (needs ~100k deals = ~15 h per probe at this search speed), so the
+-150 target was only reachable for the B reference and h2h. (4) Default search is slightly worse than B in 6-max (-118 +- 41 mbb/game
over 180k hands), consistent with the earlier finding that MC search's value over this blueprint is roughly nil; this is not caused by translation.
Caveat: default `BelowMenu::Check` leaves ~desync after tiny bets, where search falls back to no-prior/no-update; `--below smallest`
in `Blueprint.opts` would avoid that but was not measured.
Repro: `target/release/examples/vsbp --compact data/eval/B-it20000000000-20261013T1354Z/compact.plcp --hero probe:bet=3 --field search --deals 6000` (RAYON_NUM_THREADS=20); logs on box B ~/pluribus-search/logs, dev box ~/pluribus/logs/{h2h-*,ref-*}.log.

# search: per-street ablation (20000 deals = 40k hands, HU, local blueprint, same deals/seeds in every arm, AIVAT)

mbb/game of search minus blueprint, 95% CI. Defaults are now MC-only (vector/pooled opt-in via `vector`, `vec_pooled`).
| arm | AIVAT |
|---|---|
| (e) MC-only 5k average (all streets), the default | +437.0 +- 165.9 |
| (e2) same, beliefs updated with the BLUEPRINT instead of the search average (`blueprint_beliefs`) | +418.1 +- 156.0 |
| MC flop only | +44 +- 114 |
| MC turn only | +74 +- 119 |
| **MC river only** | **+415 +- 118** |
| MC flop+turn (no river) | +65 +- 146 |
| MC turn+river (no flop) | +515 +- 148 |
| (a) vector river only (CFR+, 150 passes), rest blueprint | +62 +- 98 |
| (c) vector river + MC turn/flop 5k | +229 +- 156 |
| (b) vector turn(exact infosets, 1000 passes)+river(300), flop blueprint | +75 +- 107 |
| (d) pooled flop+turn+river, Linear CFR 300/300/150 | +76.3 +- 113.3 |
| pooled flop+turn+river, CFR+ 300/300/150 (earlier run, same deals) | -157 +- 109 |
| pooled, CFR+, blueprint beliefs | still running at time of writing (logs/searchfix-armsB.log) |
MC river-only vs iterations/settings (diagnosis): 5k +415 +- 118; 20k +13 +- 96; 100k +90 +- 80; 500k +119 +- 75;
5k with warm_start 0: -139 +- 108; 5k with focus_own 0: -23 +- 165.

## Diagnosis
The whole +437 of the MC baseline comes from the MC *river* search at 5k iterations (+415) and disappears as the solve converges
(20k +13, 100k +90, 500k +119, vector equilibrium solve +62..+75): it is not search quality but an exploit of the blueprint. With
blueprint warm start and `focus_own` (half the traversals fix our actual hand) the under-converged opponent nodes stay close to the blueprint,
so our hand is effectively best-responding to the blueprint. Remove either ingredient and the gain is gone (-139 / -23). This explains the
original "5k +995, 20k -78" curve: more iterations let the opponent move away from the blueprint, so the exploit shrinks toward the
(small) equilibrium-solve value of about +60..+120 mbb/game. Replacing MC rivers with the exact vector river reduces the total
(+437 -> +229) for the same reason, and neither the flop (+44) nor the turn (+74) MC search adds anything measurable.
Updating beliefs with the blueprint instead of the search average makes no difference (+418 vs +437), so the "stacked unsafe search" effect is not
visible in HU here. "Root at the current decision point" was not run (not cheap: changes the lazy search and frozen-action design).
Pooled flop/turn/river vector with CFR+ (300/300/150) is the only arm significantly below zero (-157 +- 109); the same with Linear CFR is +76 +- 113 (difference 1.5 sigma, so CFR+ pooled is not shown to be worse than LCFR, only that this arm is not better than the blueprint). Every convergent / equilibrium-style arm lands at +60..+120 +- 100: search over this blueprint is worth roughly 0..+100 mbb/game, below what 40k hands can resolve.

## Defaults chosen (strictly by measurement)
MC-only 5k average with warm start 8 and focus_own 0.5 (best measured, +437 +- 166), `play_final=false`; vector/pooled opt-in (`vector=false`). Caveat from the diagnosis: that win is an exploit of this blueprint by an under-converged warm-started solve, not a better equilibrium; the converged arms (+60..+120) are the honest estimate of search value. Recommended for any robustness claim: vector river (`vector=true, vec_min_street=River`), which is +62 +- 98 and ~exactly solved.

## Memory (68 GB RSS in the 20k-deal pooled CFR+ run) - NOT reproduced, cause unknown
Flat RSS (1.2 GB) over 3000-deal runs of the same configuration at 24 threads and 700 s, and again in the later 20k-deal pooled LCFR arm
(d) (largest vector storage of any single search: 5 MB; turn-exact arm (b): 220 MB). So my first hypothesis (pooled nodes of 500 buckets
blowing up on deep raise trees) is NOT supported. Remaining suspects: allocator fragmentation under 24 threads over a multi-hour run, or the
27 s outlier search seen in that run. Mitigations added: `VecParams::max_bytes` (default 2 GiB per search; passes done so far are kept) and
`Stats::max_vec_mb` (printed on the eval `timing:` line) so the next long run shows whether any single search is large; if RSS grows again run with
`MALLOC_ARENA_MAX=2`. No leak found in code reading: per-hand state (`SearchPlayer`, `Arc<Solution>` cells, vector `Core` caches) is dropped at hand end.

# search: Pluribus real-time search - results (UPDATED: solver correctness fix pass)

Everything below the "Superseded" line is the earlier write-up (kept for the record). Reproduce: `examples/gate.rs`
(exact exploitability gates), `examples/eval.rs` (duplicate + AIVAT matches, LBR), `tests/vector.rs`, `tests/search.rs`.
Dev box, shared; timings are wall-clock single-thread per search under load (indicative).

## Root causes of "more iterations => worse"
1. **MC external-sampling LCFR is nowhere near converged at 5k-20k iterations.** Exact river exploitability (1326 hands, same
   tree, referee-validated BR) of the MC solution: 5k it 100-190% of pot (worse than uniform), 20k it 40-70%, 100k it 10-18%, 1M it
   0.3-1.9% (average) and 5M 0.1-0.7%. The **final iterate never converges** (1M it: 5-47%, 5M: 9-26%). So at 5k/20k the
   strategy is noise; what the head-to-head measured was blueprint warm start + `focus_own` + translation exploitation, not an
   equilibrium solve. More iterations move the noise, not monotonically toward quality. (The MC code itself is correct: its
   average strategy converges as 1/sqrt(T).)
2. Final iterate is the wrong thing to play at our budgets for ANY of our solvers (vector CFR+ final 10% vs average 0.6% at 1000
   passes; LCFR final 40-70%). Default is now `play_final = false`.
3. Without `focus_own` the MC flop loses (-375 +- 320 vs strong blueprint, 6000 deals vs +464 +- 262 with focus 0.5): the MC
   "gain" is hand-concentration of samples, i.e. a noise-reduction trick for the hand actually held, and it fades as
   iterations grow (5k +464, 20k +261, 50k +172, all +-230).
4. No bug found in beliefs / leaf perspective / frozen actions / pruning: subgame value under the blueprint equals the
   blueprint value (test `subgame_value_under_blueprint_equals_blueprint_value`: 3-handed flop, blueprint leaves, pinned
   strategies, 600k iterations: subgame -> seat0 +0.3 / seat1 -43.4 / seat2 +42.9 vs blueprint rollouts -0.1 / -44.2 / +41.6, se ~3.5).

## Fix: vector-form Linear CFR / CFR+ (`src/vector.rs`)
Heads-up subgames (any 2 live players, dead money respected) are solved by a vector solver: reach vectors over all live hole pairs,
card removal by inclusion-exclusion, stratified public chance sampling (one board card per chance node and pass), exact-hole root
round; `SearchParams::{vector, vec_min_street, vec_iters [flop,turn,river], vec_plus, vec_pooled}`. With `vec_pooled` the rounds
after the root round use bucket-pooled infosets (blueprint buckets, shared over all board runouts, regrets weighted by root belief),
which is what makes turn/flop converge in hundreds of passes. Multiway (>2 live) stays MC LCFR with depth-limited cont4 leaves.
The solver is verified against the referee: vector CFR+ average strategy has the *same* exploitability as the referee's CFR+ at
equal iterations (10.023 / 1.781 / 0.286 % of pot at 50 / 200 / 1000 it on the same spots) and our exact BR equals the referee's to
4 decimals on MC strategies. LCFR (no floor) is worse than CFR+ in this vector form (e.g. 1.9% vs 0.6% at 1000 passes); `vec_plus`
is therefore a recommended setting: **default should be CFR+ (set `vec_plus: true`)**.

## Gate 1: river exactness (exploitability, % of root pot, mean over spots; blueprint ranges at real spots)
| solver | iters | tiny-bp spots (6) | local-bp spots (8) |
|---|---|---|---|
| blueprint (translated) | - | 45.1 | 49.2 |
| MC LCFR avg (old) | 5k / 20k / 100k / 1M / 5M | 125-130 / 44-49 / 9.5-10 / 0.3-1.9 / 0.1-0.7 (4 spots, mixed) | - |
| vector CFR+ avg | 100 / 300 / 1000 | 4.6 / 1.5 / 0.57 | 5.5 / 1.8 / 0.64 |
| vector LCFR avg | 100 / 300 / 1000 | 12.3 / 6.3 / 1.9 | 14.1 / 4.1 / 1.4 |
| vector CFR+ final | 1000 | 7.4 | 10.1 |
Cost: vector 300 passes ~0.15-0.2 s; MC 1M iterations ~1.6 s on 32 threads. PASS (<1% at 1000 passes, monotone).

## Gate 2: turn exactness (4 spots, river enumerated exactly in the BR; local blueprint, blueprint = 48.9% of pot)
| vector solver | 100 | 300 | 1000 passes | time at 1000 |
|---|---|---|---|---|
| exact infosets, CFR+ avg | 171.1 | 74.7 | 27.4 | 3.8 s |
| exact infosets, LCFR avg | 167.5 | 93.6 | 47.9 | 3.9 s |
| **pooled river infosets, CFR+ avg** | **36.7** | **16.7** | **10.2** | 4.7 s |
| pooled CFR+ final | 51.9 | 32.4 | 18.9 | |
Monotone decreasing; pooled beats the blueprint already at 100 passes (0.44 s). Exact infosets need ~3000+ passes (9% pot at
3000 on the tiny-bp spots) because each river card is visited only 1/46 of the passes. PASS (documented abstraction floor).
Flop (1 spot, tiny bp; blueprint 134%): pooled CFR+ avg 20 / 60 / 200 passes = 211 / 107 / 49% (0.4 / 0.8 / 2.5 s); exact infosets
~300% (not usable). The flop vector solver is therefore pooled only.

## Gate 3: depth-limited (multiway) flop
(b) value-of-subgame-under-blueprint unit test: PASS (above). (a)/(c) cont4 vs blueprint leaves and iteration scaling in 6-max were
NOT re-run in this pass (see gaps). The multiway MC solver is unchanged apart from diagnostics (`Solution::root_value`).

## Gate 4: head-to-head vs a stronger blueprint (HU, `local` sizing, 90M iterations, 200 buckets, duplicate + AIVAT, 6000 deals = 12k hands)
Blueprint: `data/hu-local/final.bin` on the dev box (3.67M infosets, 13 min). mbb/game, search minus blueprint, 95% CI:
| configuration | AIVAT |
|---|---|
| MC only (old), 5k avg | +492 +- 304 |
| vector turn+river (exact river infosets) 300/150, flop = blueprint | +48 +- 173 |
| vector turn+river pooled 100/50, flop = blueprint | +42 +- 206 |
| vector turn+river pooled 300/150, flop = blueprint | +100 +- 187 |
| vector turn+river pooled 1000/300, flop = blueprint | +41 +- 179 |
| vector pooled flop 100 + turn 300 + river 150 | +115 +- 224 |
| vector pooled flop 300 + turn 300 + river 150 | +164 +- 215 |
| turn/river vector 300/150 pooled + MC flop 5k avg (focus 0.5) | +464 +- 262 |
|   same, MC flop 20k / 50k | +261 +- 233 / +172 +- 223 |
|   same, MC flop 5k with 500-bucket per-flop search abstraction | +578 +- 251 |
|   same, MC flop 5k, focus_own = 0 | -375 +- 320 |
Reading: all configurations are within ~1-2 sigma of each other and of 0..+500; vector budgets are flat-to-rising (no degradation
with passes, flop 100 -> 300 passes +115 -> +164); the MC flop curve is flat-to-falling with iterations (+464/+261/+172), as
before. We cannot claim a statistically significant search gain over this blueprint at 12k hands (CI +-200); 20k-deal
runs: MC-only baseline (old solver, 5k avg) at 20000 deals = +437 +- 166 (95% CI +271..+603). **The 20000-deal run of the new defaults (flop/turn/river 300/300/150, CFR+, pooled, average) came out NEGATIVE: -157 +- 109 mbb/game (95% CI -267..-48)**, mean search 3.9 s (max 27 s, shared box). So the new defaults do NOT beat this blueprint; the earlier 6000-deal rows (LCFR, +-200) were too noisy to show it. Unresolved: whether the loss comes from the pooled flop/turn abstraction floor (still 10-17% of pot exploitable vs the blueprint's ~49% in isolated-subgame gates, but nested re-solving with unsafe beliefs may still lose), from CFR+ vs LCFR in the pooled flop, or from belief/unreached-hand fallbacks. Next step: per-street ablation (river-only, turn+river, flop-only) at >=20k deals.

LBR (weak exploiter, 1500 deals x 300 samples, mbb/game won by LBR, lower = less exploitable): blueprint +842 +- 716; new search (average strategy) +234 +- 955; same search playing the FINAL iterate +1855 +- 1019. Noisy, but directionally: average beats final by a wide margin and search is not more exploitable than the blueprint.

## Chosen defaults (changed in `SearchParams`) - NOTE: gate 4 did not show an improvement over the blueprint; see the 20k-deal result above
`play_final = false` (average), `vector = true`, `vec_min_street = Flop`, `vec_pooled = true`, `vec_iters = [300, 300, 150]`
(flop/turn/river passes; ~2 s / 1 s / 0.15 s per search single-thread), `vec_plus = true` (CFR+; the 6000-deal rows above used LCFR, the 20k-deal rows below use CFR+), warm start / focus_own unchanged for the multiway MC solver.

## Remaining gaps
- Statistical power: +-200 mbb at 12k hands; the head-to-head cannot yet separate search from blueprint. Need >=100k deals or LBR/exploiter.
- 6-max (multiway) not re-evaluated; MC solver there is still noise-limited at 5k-20k iterations (use vector only when HU).
- flop is pooled-only (abstraction floor), exact-card flop needs 10^4 passes.
- Cont4 vs blueprint leaves ablation and iteration scaling for depth-limited multiway flops (gate 3 a/c) still open.
- Exploiter (ABR) cannot target a search policy (it needs a stored strategy); LBR is the only exploitability probe (weak).

---
# Superseded earlier write-up (kept; numbers below were produced by the noisy MC solver)
# search: Pluribus real-time search (Algorithm 2) - results and deviations

Reproduce on the dev box: `sharingan-1/search/run_experiments.sh` (uses `examples/eval.rs`; `examples/speed.rs` is a per-street
iterations/s probe). Tests: `cargo test --release -p search` (19 tests, <5 s). All numbers: 32 threads, dev box.

## Design
- `beliefs`: per-seat distribution over 1326 holes, card removal, Bayes. Test: equals bench `BlueprintPolicy` likelihoods to 1e-5.
- `tree`: search action abstraction (default first raise {0.5,1,2} pot + all-in, 2nd {1}+all-in, then all-in; <=4 raises),
  off-tree actions added per public node (`Extras`), paper depth rule (`limit_for`): preflop -> end of round; flop with >2 live
  players -> next street or right after the 2nd raise action; else end of game.
- `solver`: MC Linear CFR, external sampling, hogwild, seed per iteration, exact-hole infosets in the root round,
  blueprint buckets later; deals sampled from the joint belief by rejection (exact card removal); frozen own actions (our actual
  hand only, pinned in the table); leaves = 4 continuation strategies (blueprint, fold/call/raise x5), infoset = (public leaf, own
  bucket), value = rollout of the blueprint on the sampled deal, each continuation compressed to one hash-sampled action per
  abstract infoset; exact showdowns when solving to the end.
- `policy`: `SearchPolicy: PolicyFactory`, `SearchPlayer: Player`; Algorithm 2 via observe/observe_board/dist (lazy search at the first
  `dist` of a round because hole cards arrive only there); re-search from the same root on an off-tree opponent raise; preflop search
  when an opponent raise is >1 BB (=$100) from every blueprint size with <=4 players left. `dist(view, hole)` answers for any hole
  (range strategy; AIVAT/LBR). `fork()` shares the round's solution and never re-searches (off-tree => blueprint fallback).
  Play final iterate (default) or average: `SearchParams::play_final`; beliefs always use the average.

## Results (HU, blueprint = 3M it, tiny sizing {1 pot, all-in}, 200-bucket tables; duplicate, AIVAT, 20k deals = 40k hands)
Search vs its own blueprint, mbb/game (95% CI half-width):

| search | iters | play | AIVAT mbb/g |
|---|---|---|---|
| warm 8 | 5000 | final | +305 +- 153 |
| warm 8 | 5000 | average | +995 +- 179 |
| warm 8 | 20000 | average | -78 +- 144 |
| warm 8 | 20000 | final | -130 +- 123 |
(earlier 1M-it blueprint, 5000 it: no warm start -392 +- 422 (3k deals) / -580 +- 159 (20k); warm 2 +195 +- 152; warm 8 +552 +- 158; warm 32 +876 +- 158;
average +814 +- 171.) Pure blueprint through the search player: exactly 0.

LBR (HU, 300 samples, weak exploiter; mbb/g won by LBR, so lower = less exploitable):
blueprint +1365 +- 212 (15k deals); search 5000 it average +328 +- 301 (15k deals). With 20000 it average (1500 deals): -692 +- 978.
So search is less exploitable than the (tiny, translation-prone) blueprint under this LBR, and wins head to head at 5k it.

6-max, 20M-it blueprint (tiny sizing), 1 search seat vs 5 blueprints, duplicate (6 rotations), AIVAT:
5000 it average +301 +- 280 (1500 deals); 20000 it average +249 +- 290 (1000 deals); seat-0-only 3000 deals +412 +- 620.

Timing (mean per search, loaded box, single-thread solve per deal): 5000 it ~55-70 ms (HU flop/turn/river; ~25k infosets),
20000 it ~180 ms (HU), 570 ms (6-max in a nested-parallel run; max latencies of 20-35 s there are rayon nested-steal artefacts of
`--par-search` inside a match harness that already uses all cores, not solve cost). Single-thread 5000 it on a flop root: ~0.03-0.1 s.

## Caveats / honest reading
- More iterations did NOT beat the blueprint more (5k -> 20k flips +995 to -78): the head-to-head edge at low iterations is
  largely warm-start-to-blueprint plus search's extra bet sizes against a blueprint that only knows {1 pot, all-in} (translation
  exploit), not evidence of equilibrium-quality solves. LBR is the more trustworthy figure; it is noisy and weak.
- Final iterate is clearly worse than the average at 5k iterations in our MC solver (+305 vs +995); the paper's claim needs far more
  iterations/vector CFR. Default `play_final=true` follows the paper; use `play_final=false` for our budgets.
- 6-max edge is positive (~+250..400) but only ~1-2 sigma. Leaf mode ablation (cont4 vs blueprint-leaf) was not run for 6-max.
- Preflop-search path and off-tree re-search are unit-tested but not exercised in the match numbers (opponents play on-tree sizes).

## Deviations from the paper
1. Solver is MC LCFR only (no vector-form CFR); 5k-20k iterations vs the paper's seconds on 28 cores.
2. Warm start: new infosets' regrets = `warm_start * t * pot * sigma_blueprint` (default 8) - not in the paper; needed at our budget
   (without it search loses to the blueprint).
3. Negative-regret pruning inside the subgame (95% of iterations after the first fifth) and a coarser `later_sizing` for rounds after the root round.
4. `focus_own`: half of our own traversals fix our actual hole cards (unbiased per infoset, faster convergence for the hand played).
5. Later-round card abstraction = blueprint's 200 buckets (board-agnostic keys, like the blueprint), not 500 per-flop clusters (an open experiment).
6. "Second raise" counts the opening bet as the first raise action (engine `raises_this_street`).
7. Beliefs are independent per-seat marginals (joint card removal handled at sampling); own actual hand floored at 1e-6; a Bayes update with zero total likelihood is skipped.
8. LBR/AIVAT: `dist` for non-actual holes comes from the same solution (own earlier actions frozen only for the actual hand), so it is not strictly a pure function of public history.
9. Search runs lazily at the first `dist` of a round (hole cards known only then); forks may run a round's first search without focus.
10. `cargo fmt` was not run (no compute on the Mac).

## Left to do
Vector-form CFR for river/turn, per-flop 500-bucket clustering, wider preflop/off-tree match tests with off-tree-sizing opponents,
6-max cont4-vs-blueprint leaf ablation, tuning warm_start/iterations against LBR rather than head-to-head, larger LBR.

## Final-bot config: turn+river-only search (`--set flop=0`), C-final blueprint (sizing medium), 2026-10-14

Same-seed comparisons (6-max, duplicate + AIVAT, mbb/game, hero vs 5 x field; blueprint = C-final compact alone):

| field | search default (flop on) | search flop=0 | blueprint |
|---|---|---|---|
| 5 x C-final blueprint (30k, seed 2027) | +78.0 ± 31.0 | **+87.4 ± 26.5** | 0 (self) |
| chart6 (7k, seed 4104) | +15.9 ± 76.9 | +59.4 ± 70.2 | +96.9 ± 68.0 |
| chart6-loose (7k, seed 4105) | +31.6 ± 77.7 | +108.2 ± 71.3 | +123.5 ± 71.8 |
| chart6-tight (7k, seed 4106) | −2.7 ± 67.3 | +65.8 ± 62.7 | +124.9 ± 61.2 |
| RLCard DQN, retrained on B (5k, seed 4202) | +1012 ± 196 | +709 ± 214 | +476 ± 227 |

Reading: flop search is where EV leaks against off-model opponents (rule bots): flop=0 recovers most of the gap to the
blueprint vs the chart bots (mean −37 vs −120 for full search) while keeping the full gain vs blueprint-like play and
beating the blueprint vs the DQN. Belief fixes did not help (bpbeliefs=0: +42.9; tremble ε=0.05 +16.0, ε=0.2 −17.2 vs
chart6). The live Open Poker bot runs `--policy search:<C-final> --set flop=0`.
