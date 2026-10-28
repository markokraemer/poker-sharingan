# SHARINGAN development log

A condensed, chronological log of how the reproduction was built: gates, results, bugs and the fixes that mattered.
It is derived from the internal working plan. Entries about renting/archiving/deleting cloud machines, credentials and
personal environment details have been removed or scrubbed; some "box A/B/D/dev" mentions remain and just mean "the
training/eval machines". Dates are 2026. Numbers in this log are working numbers at that time; the final, curated
results are in [RESULTS.md](RESULTS.md).

---

# Plan and state (original working plan)

Goal: rebuild Pluribus (Brown & Sandholm, Science 2019) as faithfully as the public
paper + supplementary allow (not redistributed here), and measure it as rigorously as possible
without access to human pros.

This workspace is a clean rebuild on the paper's actual algorithms (an earlier heuristic-guard attempt was discarded).

## Paper spec (from supplementary; source of truth)

- Game: 6-max NLHE, 10,000 stacks, 50/100 blinds, stacks reset each hand, button rotates.
- Blueprint: external-sampling MCCFR + **Linear CFR** discount `d = (t/10)/(t/10+1)` every
  10 min for first 400 min; **negative-regret pruning** after 200 min in 95% of iterations
  (threshold C = -300M; never prune river actions or actions leading straight to terminal);
  regrets `i32` floored at -310M; lazy allocation per action sequence (preflop up front);
  preflop uses **average** strategy (counter `φ` incremented by sampled action every
  10,000 iterations, after 800 min); postflop uses the **average of current-strategy
  snapshots** taken every 200 min after 800 min. Algorithm 1 in the supplementary.
- Action abstraction: 1–14 raise sizes (pot fractions) preflop; coarser flop; turn/river
  first raise ∈ {0.5×pot, 1×pot, all-in}, later raises ∈ {1×pot, all-in}. Fold/call always.
  Exact preflop/flop size menus were NOT published → our proxy, documented and tunable.
  664,845,654 action sequences in the tree, 413,507,309 ever visited.
- Info abstraction: preflop lossless (169); flop/turn/river 200 buckets each, k-means on
  potential-aware / EMD features (imperfect recall).
- Real-time search (Algorithm 2): always on flop+; preflop only if opponent bet is >$100
  off-tree and ≤4 players remain (else randomized pseudo-harmonic translation).
  Root = start of current betting round (nested unsafe search), belief distribution over
  1326 hole pairs per player updated by Bayes from σ (blueprint or previous search).
  Our own already-taken actions frozen for our actual hand only. Off-tree opponent action
  → add to subgame tree, re-search from root.
  Depth limit: preflop search ends at end of round; flop with >2 players at round start
  ends at start of turn OR after the 2nd raise; otherwise to end of game.
  At leaves each remaining player picks one of 4 continuation strategies (blueprint,
  fold×5, call×5, raise×5 renormalised), per infoset; compressed to one sampled action per
  abstract infoset. Search abstraction: lossless current round, 500 buckets later rounds
  (per-flop clustering), 1–6 raise sizes. Solver: MC Linear CFR for big/early subgames,
  vector Linear CFR (one board sample per thread) otherwise. Play the **final iterate**;
  update beliefs σ with the **average**.
- Hardware: 64 cores, <0.5 TB RAM, 8 days for the blueprint; play on 28 cores / 128 GB.
- Eval: AIVAT (multi-player variant), mbb/game, 10k hands per format.

## Local hardware

M4 Max, 14 cores (10P+4E), 36 GB RAM, ~120 GB free disk. A paper-scale blueprint
(<0.5 TB RAM) does not fit locally → Phase 1 trains a reduced-size blueprint locally
(same algorithm, smaller action tree); Phase 2 = paper-scale run on a rented
large-memory box (needs user approval for the spend).

## Workspace layout (Rust)

| crate | owner | status |
|---|---|---|
| `cards` | core | DONE — cards, RNG, evaluator (exhaustively verified over 133,784,560 hands) |
| `iso` | agent | Waugh hand isomorphism indexer |
| `cfr-lab` | agent | Kuhn/Leduc (2p, 3p), CFR/CFR+/LCFR/ES-MCCFR (Alg. 1), exact exploitability |
| `nlhe` | agent | fast 2–6 player NLHE state machine, side pots, action abstraction, translation |
| `abstraction` | wave 2 | equity histograms, EMD k-means, OCHS, bucket tables (200 / 500) |
| `blueprint` | wave 2 | parallel lock-free MCCFR-P trainer, lazy alloc, snapshots |
| `search` | wave 3 | nested unsafe depth-limited search, 4 continuation strategies, beliefs |
| `bench` | wave 3 | AIVAT, LBR, duplicate matches, opponent gauntlet, Slumbot/GTOW/OpenPoker adapters |
| `pluribus` (bin) | wave 3 | CLI: train / play / bench |

## Gates (each must pass before building on top)

1. `cards`: exhaustive evaluator counts. ✅
2. `iso`: sizes 169 / 1,286,792 / 13,960,050 / 123,156,254 (imperfect-recall) and
   55,190,538 / 2,428,287,420 (perfect-recall, matches paper); index↔unindex roundtrip.
3. `cfr-lab`: Kuhn value −1/18 and exploitability → 0; Leduc exploitability curve for
   CFR, CFR+, LCFR, ES-MCCFR(Alg. 1); 3-player Kuhn NashConv ↓. Proves the regret code.
4. `nlhe`: rule fuzzing (chip conservation, legal-action invariants, side pots) + replay of
   known hand histories.
5. `abstraction`: bucket-quality metrics (within-bucket equity variance), determinism.
6. `blueprint`: on a tiny NLHE variant, measure exploitability vs exact BR; on full game,
   head-to-head vs earlier checkpoints and LBR exploitability lower bound must drop.
7. `search`: Leduc/small-NLHE test where search over the blueprint lowers exploitability;
   full game head-to-head search-vs-blueprint must be positive with AIVAT.
8. External: Slumbot (HU), GTO Wizard benchmark API (HU, AIVAT), Open Poker (6-max live).

## Measurement ladder (how we know it's good without pros)

1. Exact exploitability in small games (Kuhn, Leduc, tiny NLHE) — ground truth.
2. **LBR (local best response)** on the real game — lower bound on exploitability; the
   standard way to measure huge-game bots (Lisý & Bowling 2017). Also used 6-max.
3. Opponent gauntlet, duplicate + AIVAT: blueprint checkpoints, Pluribus' own fold/call/
   raise-biased blueprints, heuristic archetypes, exploiters trained against us.
4. Heads-up external near-GTO bots: Slumbot API, GTO Wizard benchmark API (AIVAT built in).
5. 6-max external: Open Poker live arena (credentials exist in a private env file).
6. Spot benchmarks: PokerBench 6-max solver spots; postflop HU spots vs an exact solver.
7. Humans (the paper's real test) — out of scope unless recruited.

## Log

- 2026-10-12: workspace created; `cards` done and verified.
- 2026-10-12: `abstraction` equity core done (exact river equity w/ card removal = brute force;
  OCHS clusters; runout histograms; EMD/L2 k-means). `blueprint::store` done (lazy i32
  regrets, floor −310M, hogwild atomics, sharded index, LCFR discount). Wave 1 agents
  running: iso, cfr-lab, nlhe, BENCHMARK_LANDSCAPE.md research.
- NEXT: when nlhe lands → write blueprint traversal (Alg. 1) myself; when iso lands →
  bucket-table builder (flop/turn EMD 200, river OCHS 200). Wave 2 agents: bench (AIVAT,
  duplicate, gauntlet, LBR), search (Alg. 2) on Leduc first.
- 2026-10-12: BENCHMARK_LANDSCAPE.md written (research agent). Key: no public 6-max near-GTO bot API
  exists; GTO Wizard benchmark = HUNL 200bb, free key via manual form (USER ACTION needed),
  100k hands/mo, AIVAT built in; Slumbot API up; Open Poker = free 6-max bot arena (10/20).
- 2026-10-12: GATE 3 PASSED (verified): cfr-lab 9/9 tests. Kuhn P1 value −1/18; Leduc CFR+
  5k → 0.015 mbb-chips expl; Alg.1 MCCFR-P Leduc 1M iters → 40.5 milli (avg), current
  iterate 243 milli (so: average matters; Pluribus postflop = avg of snapshots).
  Wave 2: search-lab agent (Alg. 2 on Leduc with exact exploitability) launched.
- 2026-10-12: GATE 4 PASSED (verified): nlhe 20/20 tests (1M-hand fuzz, side pots, short
  all-in rule). apply ≈ 9 ns. Our proxy blueprint tree = 1,544,632,126 decision nodes
  (paper 664,845,654) — preflop/flop menus somewhat larger than Pluribus'.
- 2026-10-12: blueprint trainer (Alg. 1) written: traverse/MCCFR-P, UPDATE-STRATEGY,
  LCFR discount, pruning, checkpoints (`blueprint::io`). BUG FOUND+FIXED: per-thread RNG
  seeding replayed identical deals across rayon splits → now one RNG per iteration.
  HU sanity: AA calls shove, 72o folds. Throughput 6-max (14 cores): tiny sizing ≈73k it/s
  (9M nodes/s); paper sizing ≈20k it/s but 15M infosets after 82k it → local run must use a
  reduced sizing; paper-scale needs the big-memory box.
  Wave 2: bench agent launched (Policy API, gauntlet, duplicate+AIVAT, LBR, Slumbot).
- 2026-10-12: GATE 5 PASSED: bucket tables built (data/buckets-200; flop/turn EMD-histogram,
  river OCHS-8; R² of equity 0.93/0.95/0.96; ~2 min build). search-lab (Leduc, exact):
  R2 re-solve cuts expl 3.5×; cont4 leaves ≫ blueprint leaves (31 vs 585 milli); stacked
  R1+R2 unsafe search hurt vs R2-only in Leduc (watch this in NLHE). bench crate done
  (Policy API, gauntlet, duplicate+AIVAT 2–6× var. reduction, LBR, Slumbot client — not run).
- Store redesign: per-infoset alloc + lock-free open-addressing index (per-sequence blocks
  were ~10× worse: 6-max reaches each history with few buckets). Postflop snapshots +
  `pluribus export` (φ preflop + snapshot-average postflop → regrets=probs store).
- 2026-10-12 ~19:35: RUN RESTARTED with live eval (pluribus::live_eval): every 60 min the
  current blueprint plays 2000 duplicate deals × 6 seats vs tag/lag/nit/station/maniac and
  its own fold/call/raise×5 variants, AIVAT-scored → ~/pluribus/data/run-paper/eval.jsonl.
  First point (9.3M it, 6 min): −2051 vs tag, −1218 vs nit, +1582 vs station (untrained).
  Built-in bots are SANITY ONLY. "Worthy" evaluation = Wave 4 (launched):
   (a) crates/exploit — trained best-response exploiter (MCCFR learner vs frozen blueprint)
       + strong LBR: opponent-independent exploitability, the GTO yardstick;
   (b) crates/referee — PokerBench 6-max spots labelled by GTO Wizard solutions (accuracy)
       + exact HU river solver EV-loss;
   (c) external: GTO Wizard AI API (HU; USER must request key), Slumbot (HU), Open Poker 6-max.
- 2026-10-12 ~21:30: SPEED: perf showed a global per-node AtomicU64 (128 threads) + no THP
  dominating → thread-local counter + madvise(MADV_HUGEPAGE): ~31k → ~51k it/s and rising
  (restarted run, attempt 3). HistKey (8-byte hash-only history) done, NOT yet deployed.
  exploit crate DONE (ABR exploiter: HU toy curve 11.1→3.3→2.5 bb/g as blueprint trains;
  push/fold vs exact eq. −9.5±1.7 mbb, vs perturbed +222 (exact 226)).
- 2026-10-12 ~23:00: referee crate DONE + verified on dev box (7/7): PokerBench 574,200 spots
  (GTO-Wizard-labelled, Apache-2.0 per HF) replay 100%; baselines exact-acc nit .645 / tag .512
  / station .500 / toy-bp .450 (modal-label metric → regression signal only). Exact HU river
  CFR+ referee validated on analytic toy (bluff 50%, call 50%); EV-loss baselines tag 11.1k,
  station 22.1k mbb/spot. exploit crate tests pass on dev box.
- 2026-10-12 ~23:30: search crate v1 DONE (19 tests on dev box): beliefs (Bayes, card removal),
  Alg. 2 (frozen own actions, off-tree re-search, preflop trigger), MC-LCFR with cont4 leaves,
  depth-limit rule. RED FLAG: HU vs own blueprint +995 mbb @5k iters (avg) but −78 @20k →
  more iterations worse ⇒ bug/formulation issue. Wave 5 launched (dev box only):
   (a) search correctness hill-climb: river/turn exploitability-vs-iterations gate using the
       referee's exact BR; vector-form LCFR for river/turn (paper); then iteration curves;
   (b) per-flop potential-aware EMD 500-bucket search abstraction (paper) wired into search.
  Training attempt 4 @40 min: ~90k it/s instantaneous, 4.0B infosets (23% cap).
  Primary scoreboard = 6-max: 1 bot vs 5 opponents, 5 bots vs 1 trained exploiter (paper's
  1H+5AI analogue), AIVAT; HU (Slumbot/GTOW) secondary.
- 2026-10-12 ~23:55 LEARNING CURVE (6-max, hero vs 5× opp, AIVAT mbb/game):
  it 11.6M: tag −1835 | lag −1978 | nit −1008 | station +1897 | maniac −1258
  it 297M (~1h): tag −3 | lag −180 | nit +36 | station +2191 | maniac −1039 | biased self ≈0
  Throughput 76k it/s avg; 5.05B infosets (30% cap) @70 min; pruning from 347M.
- 2026-10-13 ~00:50: per-flop 500-bucket search abstraction DONE (verified on dev box, 10/10):
  1,755 flop tables (6 GB, data/search-buckets-500 on dev box), river OCHS L2, turn
  potential-aware (hist over river buckets, approx EMD tree+1D). River R² .9976 vs .9897
  (blueprint-200). Opt-in SearchParams::search_buckets; play impact not measured yet.
- 2026-10-13 ~01:50 A/B: A(paper) @938M: tag +205 lag +242 nit −46 station +1850 maniac −1072.
  B(local) @65M: tag −22 lag +637 nit −123 station +2315 maniac +1400. B 235k it/s (A 85k),
  B 6% mem vs A 45%. Hypothesis: A's 14-size preflop menu spreads samples thin → undertrained
  infosets vs aggressive/off-tree bets. Decide at matched iterations (~1B) next eval; if B ≥ A,
  repurpose box A for variant C (menu hill-climb).
- 2026-10-13 ~02:25: A @1.27B: tag +92 lag −6 nit +9 station +1737 maniac −897 (47% mem).
  Heuristic gauntlet too noisy to rank A vs B (±200–350) → launched eval-pipeline agent:
  streaming checkpoint→compact policy (no full load), CompactPolicy, A-vs-B 6-max h2h
  (3v3, 1v5 both ways, AIVAT), exploiter vs compact, scripts/eval_checkpoint.sh.
- 2026-10-13 ~02:45 MATCHED-ITERATION A/B (6-max mbb/game): A(paper)@1.27B tag +92 lag −6 nit +9
  maniac −897 station +1737 | B(local)@0.97B tag +209 lag +743 nit +111 maniac +1498 station
  +2310 → B better everywhere, 2.9× faster, 5× less memory. PLAN: keep A until its 6 h
  checkpoint (for a real A-vs-B h2h via eval pipeline), then repurpose box A → variant C
  `--sizing medium` (8 preflop opens; presets now single-sourced in bench::bp::sizing_by_name).
- 2026-10-13 ~03:15 BUG (perf): at avg_after (1.389B it) UPDATE-STRATEGY enumerated every
  opponent preflop action → A −62% (66k→25k it/s), B −35% (273k→176k), and allocated
  ~850M junk infosets on A. FIX (trainer.rs, tested on dev): skip opponent actions with σ=0
  exactly (unreachable now → zero weight) and lookup-only (no allocation). Deploy: A → at
  its 6 h checkpoint keep ckpt for h2h, start variant C (--sizing medium) fresh; B → resume
  from its first checkpoint with the new binary.
- 2026-10-13 ~03:50 SEARCH GATES (verified 23 tests on dev): MC ext-sampling LCFR correct but
  ~1/√T: river expl 100–190% pot @5k, 0.3–1.9% @1M → v1 was unconverged, not buggy. NEW
  vector-form CFR+ (search/src/vector.rs, exact BR matches referee): river 49%→0.64% pot @1000
  passes (~0.15 s @300); turn pooled 48.9%→10.2% @1000. Final iterate ≫ worse than average at
  these budgets → play_final=false. Defaults: vector, pooled, vec_iters [300,300,150], CFR+.
  HU h2h vs blueprint (6k deals each): all configs within ~2σ (+41…+164 ±~200); MC-flop
  +437±166 @20k deals driven by focus_own. Search gain NOT yet significant; 6-max multiway still MC.
  Open: 20k-deal run of new defaults (logs/searchfix-h2h5.log), 6-max, 64 GB RSS growth in match.
- 2026-10-13 ~04:05 CORRECTION: new vector defaults @20k deals = −157±109 mbb/g vs blueprint
  (significant loss); MC-only 5k avg = +437±166. Vector solver exact per-subgame, so the loss
  is in chaining (pooled floor / CFR+ in pooled flop / beliefs after unsafe re-solve /
  rare-hand fallback). Agent resumed: revert defaults to best measured, per-street ablation
  ≥20k deals, blueprint-vs-search-average belief arm (stacked-unsafe hypothesis), fix 64 GB RSS.
- 2026-10-13 ~05:00 BUG (perf): checkpoint save funnelled every record through one Mutex<Vec>
  across 128 threads → ~54 MB/s; A's first checkpoint (~280 GB) stalls training ~90 min.
  FIX io.rs: per-range buffers (Store::entry_at), measured 618 MB/s save / 300M infosets in
  15 s load on dev box (examples/save_bench). Applies to C (new) and B (after its resume).
- 2026-10-13 ~06:05 B curve (6-max mbb/g): 1.74B tag +322 lag +918 nit +112 st +2604 man +1486 |
  2.41B +311/+1019/+154/+2669/+1835 | 3.11B +350/+964/+36/+2919/+1546 → heuristic gauntlet
  saturated (B beats all); progress now only measurable by exploiter + h2h (eval pipeline).
  C(medium) @17M already maniac +1238 → A's maniac weakness was the 14-size preflop menu.
  B 17.9% @4.75 h (~25 h ETA). Watcher armed for B's first checkpoint → resume w/ fixes.
- 2026-10-13 ~07:15 SEARCH ABLATION (HU, 20k deals, AIVAT): the +437 MC-search edge = exploit of
  our own blueprint (under-converged MC river + warm start + focus_own ⇒ best-responds to the
  blueprint): river-only 5k +415; 20k +13; 500k +119; warm_start 0 → −139; focus_own 0 → −23.
  Convergent arms ≈ +60…+120 ±100 (vector river exact +62±98). Beliefs blueprint vs search-avg:
  no difference (+418 vs +437). Defaults reverted to MC 5k avg (best measured) with vector opt-in;
  BUT the right search metric is exploitability (LBR/exploiter), not h2h vs own blueprint → TODO
  pick search config by LBR. 64 GB RSS not reproduced; VecParams::max_bytes cap added.
- 2026-10-13 ~07:40: B first checkpoint at it 4.505B (55 GB, 802 s with old save) → B RESUMED with
  all fixes: ~412k it/s instantaneous (was ~176k) → B's 2e10 ETA ≈ 10.5 h. Infoset growth now
  ~25M/min-equivalent... negligible (1.957B, 12% cap).
  Open Poker 6-max adapter agent launched (smoke ≤10 hands with builtin only; real runs = user call).
- 2026-10-13 ~09:00 *** FIRST EXPLOITABILITY: trained exploiter (12M it, 1 seat vs 5×A@1.53B, 6-max,
  AIVAT) wins +470 ±127 mbb/g avg; by seat SB −162 | BB −368 | UTG +1009 | MP +710 | CO +1068 |
  BTN +561 (learner miss 1.0%). This is THE metric to drive down (paper's 1H+5AI: humans −32.7).
  Exploiter likely still undertrained (number is a lower bound).
- 2026-10-13 ~09:10 eval pipeline FINAL: full-scale verify on B's real ckpt (1,953,658,056 infosets)
  compact vs export_in_place: 0 missing, 0 bad, max err 0.00314 (<1/255). Eval data lives in
  ~/pluribus-eval/data (remote.sh sync uses --delete on ~/pluribus).
- 2026-10-13 ~11:45 local: A RESUMING on box D from 1.5275B (checkpoint verified by size).
  Status: C(medium, box A) @3.04B (15%), 127k it/s, evals tag +151 lag +1017 nit +78 man +1978;
  B(local) @12.0B (60%), 479k it/s, tag +363 lag +1127 nit +119 man +1507 — finishes in ~4.6 h.
- A resumed on D: ~105k it/s (was ~25k with the UPDATE-STRATEGY bug), 9.67B infosets (58% cap),
  growth ~10M/min. ETA for 2e10 ≈ 49 h. B ETA ~4.5 h; C ETA ~40 h.
- ~12:45 local: exploiter vs B (identical config to A's: 12M it, all seats, 20k eval deals, 60 GB
  learner; 48 threads) running on dev box → data/eval/B-it4504691056-*/exploit-12M.json.
  Gives like-for-like exploitability A(+470±127) vs B.
- ~13:25 local: EXPLOITER vs B (12M it, learner menu = B's local menu): −205±57 (exploiter LOSES;
  by seat SB −625 BB −1079 others +67…+161). Not comparable to A's +470 (learner there had
  A's rich paper menu). Added `exploit --learner-sizing` (learner menu independent of target,
  bets translated into the target's abstraction). Re-running vs B with --learner-sizing paper
  (exploit-12M-paperlearner.json) for a like-for-like comparison.
  Runs: C(box A) 18.9%, B 72.9% (~2 h left), A(box D) 9.8% @103k it/s.
- BUG (exploit, cross-menu): learner history used the learner-menu index of opponents' actions;
  with learner≠target menus two real bet sizes shared an index → distinct states on one learner
  infoset (panic: action index OOB). FIX: learner histories hash the exact concrete action
  (learner_code), training + play. Same-menu results (A +470, B −205) unaffected. Rerunning
  vs B with --learner-sizing paper.
- ~15:05 local *** CRITICAL: exploiter vs B with richer learner menu (paper) = +26,224 ±501 mbb/g
  (by seat +12.7k/+7.2k/+51.7k/+38.8k/+29.0k/+17.9k), vs −205 with B's own menu. Off-menu bet
  sizes → B's translation is catastrophically exploitable (or a translation-path bug). This is
  why Pluribus searches on off-tree actions. Agent launched: diagnose (probe bots, EV
  attribution), fix bugs, randomized pseudo-harmonic, re-measure A/B, test SearchPolicy vs probes.
  B 91% done.
- ~16:10 local: *** B DONE: 2e10 iterations (paper's iteration budget scaled), final ckpt
  64 GB + sums. Final live eval (6-max mbb/g): tag +310, lag +788, nit +135, station +3541,
  maniac +1485; vs own biased variants −46/−5/−20. Running eval_checkpoint.sh B (compact w/
  snapshot averaging, gauntlet, h2h vs A@1.53B, 2M deals/format). Box B then = eval box.
- ~16:50 local *** B FINAL (2e10, snapshot-avg, compact 28 GB, 2.38B infosets) vs A@1.53B,
  2M deals/format, AIVAT: 3A+3B → B +14.6±2.2; 1A vs 5B → A −33.2±3.7; 1B vs 5A → B +20.9±3.6.
  B now beats A@1.53B (B@4.5B lost to it) → B kept improving to 20B. Gauntlet 6-max: tag +481,
  lag +1058, nit +143, station +3752, maniac +1547. Fair A-vs-B needs A further trained (D).
  Box B idle → offered to translation agent as 128-thread exploiter/eval box.
- ~17:15 local: C(medium, box A) @5.91B (29.5%): tag +173 lag +896 nit +63 st +2073 man +1746.
  A(paper, box D) @3.44B (17%): tag −4 lag +68 nit −6 st +1657 man −1370; 68% mem cap.
  Paper menu converges slower vs aggression. Next: h2h A/B/C at A's next ckpt (~75 min).
- ~17:30 local: USER → deploy vs openpoker.ai + strongest public bots. LAUNCHED B-final live on
  Open Poker from dev box (openpoker live --policy compact:B-final --sizing local, buy-in 2000=100bb,
  10k hands / 48 h cap; trace logs/openpoker-Bfinal.jsonl). External-bots agent launched:
  research strongest runnable NLHE bots (learned + rule-based), integrate 3–6 as PolicyFactory
  (Rust or Python subprocess bridge), 6-max duplicate+AIVAT vs B-final; train HU 200bb blueprint
  and play ≥2k hands vs Slumbot live.
- ~17:55 local: Open Poker live (B-final): 55 hands, +170 bb/100 (CI ±300), 0 rejects/desyncs,
  VPIP 16.7 / PFR 14.8, mostly 6-handed; ~110 hands/h (10k hands ≈ 90 h). A(D) @3.68B, ckpt ~40 min.
- 2026-10-13 ~18:45 local: EXTERNAL BOTS done (crates/bench/EXTERNAL_BOTS.md). No strong public 6-max bot exists;
  integrated chart6/-loose/-tight (native rule bots), RLCard DQN via JSON-lines Python bridge (`bench table`, `ext:`),
  B-final 6-max (AIVAT, 100k deals): vs 5x chart6 +134+-18, loose +174+-20, tight +130+-16, rlcard +587+-41;
  1 external vs 5 B: all lose (chart6 -112, rlcard -1779). Mixed tables: B positive but chart bots earn more than B
  when a donor sits (B is non-exploitative). HU 200bb blueprint (pluribus train --stack 20000, 700M it, 6.3M infosets)
  vs Slumbot live 2,400 hands: +13.9 +-68.7 bb/100 raw, +14.1 +-65.5 luck-adjusted (not significant).
- ~19:45 local *** A@3.99B (snapshot-avg, compact 140 GB) vs B-final (20B), 2M deals/format:
  3A+3B A −10.9±2.1 (was −14.6 @1.53B) | 1A vs 5B −21.9±3.6 (was −33.2) | 1B vs 5A B +12.8±3.5
  (was +20.9) → A closing the gap at 20% of its run. A gauntlet 6-max: tag +364, lag +693,
  nit +177, station +2031, maniac +2045 (maniac weakness gone).
- ~20:15 local *** TRANSLATION BUG FIXED (agent; verified tests nlhe/bench/exploit/openpoker on dev):
  after off-menu actions the target appended the translated index to its history but built its
  action list from the REAL state → 92% lookup misses → check/call; when lists matched, check-prob
  applied to Fold; overbets ≥3× called with everything. FIX = shadow game (nlhe::abstraction):
  track a game where off-menu raises are replaced by their translation; tiny bets (< smallest size)
  → check/call; overbets → all-in only if ≤0.5 pot behind. Exploiter (paper learner, 12M) vs
  B-final: +26,007±500 → +700±149 (genuine residual; randomized translation +619±134, n.s.).
  CONSEQUENCES: (1) Open Poker restarted on fixed binary (trace logs/openpoker-Bfinal-fixed.jsonl;
  old 155 hands −64 bb/100 were with the bug); (2) all A-vs-B h2h handicapped B (A's sizes are
  off-menu for B) → re-running A@3.99B vs B-final on fixed code; (3) search crate still has the
  old history logic → agent porting shadow game into search + re-measure vs probes on real B.
- 2026-10-13 ~20:25 local (session restarted): status — C(medium, box A) 41.2% @8.24B, 151k it/s;
  A(paper, box D) 24.7% @4.95B, 114k it/s, 12.4B infosets; B done. Fixed-code h2h A@3.99B vs
  B-final = identical to the earlier run (−10.9/−21.9/+12.8) → that earlier eval (synced 17:23 dev
  time) already contained the shadow fix; B-final still ahead. Open Poker fixed session: 35 hands
  (−124 bb/100, noise). Search shadow-port agent resumed (its probe runs still running on B/dev).
- ~21:15 local: SEARCH shadow port DONE (verified 24 tests on dev). On real B-final, 6-max duplicate+AIVAT:
  probes vs search lose −15k…−17k mbb/g (vs −7.5k vs blueprint B) → search far more robust to off-menu
  sizes; BUT search vs 5×B = −118±41 mbb/g (30k deals) → 6-max search WEAKER than its blueprint (must be ≥).
  Launched 6-max search hill-climb agent (box B): ablations per street/multiway/iterations/warm-start/
  focus_own/leaves/abstraction; vector LCFR for 3+ player late subgames; defaults by measurement.
- ~21:45 local: Open Poker bot had BUSTED at 18:29 UTC after 47 hands (−183 bb/100, noise-level) and
  sat idle: server sent `bust` {options: rebuy|leave} and removed the seat — it does NOT auto-rebuy
  (README assumption wrong). Restarted with --no-auto-rebuy (we rebuy+rejoin ourselves), trace
  logs/openpoker-Bfinal-fixed2.jsonl. Loop ticks must check trace freshness (stall detection).
  Exploiter (12M, paper same-menu) vs A@3.99B launched on dev (trend vs +470 @1.53B).
- ~22:45 local *** EXPLOITABILITY TREND (A, paper, 12M-it trained exploiter, same menu, 6-max 1 vs 5×A, AIVAT):
  A@1.53B +470±127 → A@3.99B −405±65 (exploiter now LOSES; seats SB −947 BB −1427 UTG +67 MP +80 CO +30
  BTN −232; learner miss 1.2%). Lower bound only (bigger exploiter budgets could find more).
  Open Poker (rebuy fix): alive, 111 hands +18 bb/100 (noise).
- 2026-10-14 ~00:20 local *** C (medium) @10.29B (51%; eval tag "A-it10285412720…" because it ran on box A)
  vs B-final, 2M deals/format: 3C+3B −0.9±1.9 | 1C vs 5B −4.8±3.2 | 1B vs 5C B −1.1±3.2 → C ≈ B-final
  at half its run. Gauntlet 6-max: tag +412 lag +1035 nit +163 station +2486 maniac +1856. Exploiter
  (12M) on C running on dev.
- ~01:15 local *** EXPLOITABILITY TABLE (12M trained exploiter, 6-max 1 vs 5 copies, own-menu learner):
  A@1.53B +470±127 | A@3.99B −405±65 | C(medium)@10.29B −330±66 (seats −787/−1384/+131/+147/+48/−137) |
  B-final −205±57 (own menu) / +700±149 (paper-menu learner, off-menu sizes). Running C vs paper-menu
  learner for the cross-menu comparison.
- ~01:30 local: Open Poker bot busted again at 21:29 UTC (192 hands, −39 bb/100) and idled: our rebuy
  got `cannot_rebuy — you still have chips` (season balance > 0). FIX session.rs: on cannot_rebuy →
  rejoin lobby; regression test added (openpoker 35/35 on dev). Restarted → trace
  logs/openpoker-Bfinal-fixed3.jsonl. Live total so far ≈ 240 hands with 2 busts (noise-level; watch).
  Training: C 58.7% @11.74B; A(D) 36.4% @7.29B. Cross-menu exploiter vs C at 64%.
- ~01:55 local *** CROSS-MENU EXPLOITER (paper-menu learner, 12M) vs C(medium)@10.29B: +3±98 mbb/g (seats
  −103/−747/+114/+227/+231/+296) vs B-final +700±149 → medium menu + shadow translation ≈ closes the
  off-menu hole. CURRENT LEADER for the final bot: C (≈B-final h2h at half its run, unexploitable
  own-menu −330 and cross-menu +3). A (paper) also unexploitable own-menu (−405 @3.99B).
- ~02:25 local: B-final Open Poker session (fixed3) ended at 129 hands +23 bb/100. SWITCHED live bot to
  C@10.29B compact (sizing medium; trace logs/openpoker-C10B.jsonl) — more robust to off-menu sizes.
  Search hill-climb agent active on box B (load ~85).
- 2026-10-14 ~04:00 local: C 69.0% @13.79B 168k it/s; A(D) 43.4% @8.68B, 13.9B infosets, 82% mem cap →
  hits the 85% index cap in ~2–3 h, then graceful "store full" (no new infosets). Open Poker C: 219 hands
  +4 bb/100 (±117). Search agent still running on B.
- ~05:05 local: search hill-climb interim (6-max, 1 search vs 5×B-final, 20k deals, AIVAT): turn+river-only
  +31.5±40.5 | 80k iters +6.9±41.6 | small −4.7±44.7 | default-final −38±46 | flop −51±43 | warm0/focus8
  −58±47 → search now ≈ blueprint (was −118); flop multiway search is the weak spot. Open Poker C:
  328 hands +63 bb/100 (±153).
- ~05:15 local: queued sequential evals (gauntlet + h2h vs B-final 2M/format + 12M exploiter): C@14.76B
  (ckpt 03:21 UTC) then A@9.72B (ckpt 04:10 UTC, 400 GB). Logs /tmp/claude-501/eval_C15.log, eval_A97.log.
  Open Poker C: 474 hands +28 bb/100.
- ~06:50 local *** C@14.76B (74%) vs B-final, 2M deals/format: 3C+3B +2.0±1.9 | 1C vs 5B +0.1±3.1 |
  1B vs 5C B −2.1±3.1 → C now ≥ B-final (trend −0.9 @10.3B → +2.0). Exploiter on C@14.8B at 85%.
- ~09:20 local: C@14.76B own-menu exploiter (12M) −330±62 (same as @10.29B → still unexploitable).
  A@9.72B eval running (compacting 400 GB on D). Training: C 89.6% @17.92B 178k it/s (ETA ~3 h);
  A(D) 57.3% @11.45B 130k it/s, slots 86% of cap — graceful store-full path verified in main.rs:114
  (warns, stops allocating; no panic). Open Poker C alive: 695 hands raw +78 / all-in-adj +20 bb/100
  (±180, noise; VPIP 19.5 PFR 16.5). Search agent still on box B.
- ~09:50 local: quiet tick. A@9.72B compaction pass 1 at group 6/8 (~8 min/group). C 91.8% @18.37B;
  A(D) 58.8% @11.75B (86% cap). Open Poker C: 742 hands, all-in-adj +4 bb/100 (±171). Search agent running.
- ~10:20 local: A@9.72B compacted → 166 GB PLCP1; transferring D→dev. C 93.8% @18.75B; A(D) 60.1%
  @12.03B (87% cap). Open Poker C: 789 hands, all-in-adj +16 bb/100 (±164). Search agent running.
- ~10:50 local: A@9.72B compact on dev (transfer 1402 s); gauntlet/h2h running. C 96.0% @19.20B (final
  ETA ~1.2 h); A(D) 61.5% @12.31B. Open Poker C: 840 hands, all-in-adj +25 bb/100 (±155).
- ~11:20 local *** A(paper)@9.72B (49%) eval: gauntlet random +3342 / station +2140 / nit +135 / tag +378
  / lag +691 (all >0). h2h vs B-final 2M/format: 3A+3B −3.8±2.0 | 1A vs 5B −12.9±3.4 | 1B vs 5A B +5.6±3.4
  (trend −10.9 @3.99B → −3.8). Own-menu exploiter 12M: −356±61 (unexploitable; 591M learner infosets).
  → C(medium) still the leader (+2.0 vs B @14.76B); A improving but behind at half its run.
- ~11:58 local *** C (medium) TRAINING COMPLETE: 2e10 iters in 1837 min (30.6 h), 10.52B infosets, 25.3G slots
  (67% cap), 181k it/s avg; final ckpt blueprint.bin 291 GB. Final eval launched (compact → gauntlet + h2h vs
  B-final 4M deals/format + 12M exploiter) → /tmp/claude-501/eval_Cfinal.log, tag A-it20000000000-…T0959Z.
  A(D) 64.4% @12.88B (88% cap). Open Poker C: 937 hands all-in-adj +17 bb/100 (±142).
  Plan: box A idle after compaction → resume C to 4e10 (paper trained 8 days; C still improving vs B).
- ~12:30 local: C-final compact = 125 GB (A@9.7B was 166 GB), transferring A→dev; final eval follows.
  C RESUMED on box A toward 4e10 (same args, --iters 40000000000; schedule phases lcfr/prune/avg all already
  passed under both scalings → only effect is more iterations). 2e10 checkpoint preserved as
  run-medium/blueprint-2e10.bin (hard link; saves are tmp+rename). A(D) 66.0% @13.19B, ETA ~12 h.
- ~12:40 local *** SEARCH HILL-CLIMB AGENT DONE (crates/search/RESULTS.md): 6-max search vs 5×B-final now
  +100.9±22.0 mbb/g (60k deals, seed 11) / +95.3±31.1 (new default, 30k, seed 11); was −118±41. Levers:
  (1) iterations 20k→200k (−42 → +87; 500k no better); (2) opponent beliefs updated with the BLUEPRINT
  likelihood (they are the blueprint) instead of the search average (−150 → −52 @20k); own seat keeps search
  avg; off-tree opp actions keep search avg. Parallel-solve 10× slowdown fixed (per-thread counters, own pool).
  Latency 32-thr pool: mean 237 ms / max 528 ms per decision. Probe bet=1.5 vs 5×search: −10.9k (B −7.6k,
  old search −16.3k). Exact HU vector solver doesn't help in 6-max (−78..−137) → multiway vector LCFR skipped.
  MY GATES: cargo test -p search on dev 24/24 ✓; independent replication seed 2026, 30k deals running on B
  (~/ps3/logs/hc/verify_default_s2026.log). C final-eval h2h running on dev; C resume loading ckpt (RSS 333 GB).
- ~13:05 local *** C-FINAL (medium, 2e10) eval: 6-max gauntlet random +4595 / station +2571 / nit +137 / tag
  +441 / lag +1052 / maniac +1804 (all >0; the 2-max section is a 6-max blueprint seated HU — ignore).
  h2h vs B-final 4M deals/format: 3C+3B +1.6±1.3 | 1C vs 5B +1.1±2.2 | 1B vs 5C B −1.7±2.2 → C ≥ B
  (3+3 significant at 95%). Exploiter on C-final running on dev. C resume healthy: 20.37B @ ~180k it/s
  (4e10 ETA ~31 h). Copying C-final compact A→B (~/pluribus-search/data/Cfinal) for search-on-C runs.
- ~13:25 local: C-final compact on B. Launched search-on-C (new default, sizing medium) vs 5×C-blueprint,
  30k deals seed 2027, 64 thr → ~/ps3/logs/hc/searchC_vs5C_s2027.log (final-bot candidate gate:
  search ≥ own blueprint). Replication of search-on-B (seed 2026) still running alongside.
- ~14:10 local *** SEARCH GATE VERIFIED (independent seed): search(new default) vs 5×B-final, 30k deals seed 2026:
  AIVAT +110.1±30.8 mbb/g (raw +84.9±31.1) — replicates agent's +100.9±22.0. (Harness latency mean 4.7 s is
  1-thread-per-search under 2 concurrent runs; live uses a 32-thr pool ≈ 0.24 s.)
  C-final own-menu exploiter 12M: −322.9±61.9 → unexploitable (same as @10.3B/@14.8B).
  C resume @21.51B (53.8%), ~275k it/s since resume (ETA 4e10 ~19 h). A(D) 70.9% @14.19B (89% cap).
  Open Poker C: 1266 hands, all-in-adj −31 bb/100 (±148, noise). search-on-C vs 5×C running on B.
  Launched sonnet agent: wire SearchPolicy into the openpoker live bot (`--policy search:PATH`, hard
  per-decision budget with blueprint fallback, tests, latency on C-final). Live bot untouched until verified.
- ~14:25 local: OPENPOKER SEARCH INTEGRATION (sonnet agent) DONE: `openpoker live --policy search:PATH
  --search-threads N --search-budget-ms MS --set k=v`; hard budget = worker thread + deadline → blueprint
  fallback (traced `fallback_reason`), solver soft cap 70% of budget; new `openpoker offline` mock-table mode.
  Latency on C-final (16 thr, busy dev box): search decisions mean 0.77–0.97 s, p95 ~2.1 s, max 2.8 s, 0
  fallbacks over 600 hands. MY GATE: cargo test -p openpoker -p search -p bench on dev → all pass (openpoker
  37, search 24, bench 10). Risk: release profile panic=abort → run live bot under a restart loop.
  Live switch to search waits for the search-on-C vs 5×C gate (running on B).
- ~14:55 local *** FINAL-BOT GATE: search(new default) on C-final vs 5×C-final blueprint, 30k deals seed 2027:
  AIVAT +78.0±31.0 mbb/g (raw +52.8±30.7) → search ≥ own blueprint on C too. (Search on B: +110/+101.)
  LIVE SWITCH: stopped Open Poker C10B blueprint bot (final: ~1300 hands, adj ≈ −30 bb/100 ±150, noise;
  trace logs/openpoker-C10B.jsonl). New live bot = C-final (2e10) + search, 16 thr, 5 s budget, under a
  restart loop scripts/op-search-loop.sh (binary target/opsearch-live/openpoker; touch
  logs/openpoker-Csearch.stop + kill to end). Trace logs/openpoker-Csearch.jsonl; first searched decisions
  0.2–0.3 s, 0 fallbacks.
- ~15:35 local: live search bot healthy: 71 hands +11 bb/100 (noise), 89 decisions, 0 fallbacks/rejects/desyncs,
  0 restarts; latency p50 0.09 ms (non-search) / p95 455 ms / max 654 ms. C resume 57.3% @22.90B 277k it/s;
  A(D) 75.0% @15.00B (90% cap). Launched sonnet agent on box B (~/ps4): FINAL-BOT external gauntlet —
  search-on-C vs 5× chart6/loose/tight/RLCard, 1 ext vs 5 search-C, and C blueprint like-for-like.
- ~16:05 local: quiet. Live search bot 131 hands +9.7 bb/100 (±35), 0 fallbacks/restarts. C 58.5% @23.40B;
  A(D) 76.5% @15.31B. External-gauntlet agent running on B.
- ~16:35 local: quiet. Live search bot 174 hands +117 bb/100 raw (±180), 276 decisions, 0 fallbacks/restarts.
  C 59.8% @23.91B; A(D) 78.1% @15.63B. External-gauntlet agent mid-run on B (~/ps4/logs/final).
- ~17:05 local: quiet. Live search bot 231 hands +186 bb/100 raw (±190, noise), 0 fallbacks/restarts.
  C 61.1% @24.43B; A(D) 79.8% @15.97B (91% cap). External-gauntlet agent still running.
- ~17:36 local: quiet. Live search bot 289 hands +143 bb/100 raw (±156), 452 decisions, 0 fallbacks/restarts.
  C 62.5% @25.00B; A(D) 81.6% @16.32B. External-gauntlet agent: (a)/(b) arms done, (c) blueprint arms running.
- ~18:07 local: external gauntlet interim (agent still finishing; 6-max AIVAT mbb/g, C-final compact):
  search-C vs 5×: chart6 +16±77 | loose +32±78 | tight −3±67 | rlcard +1012±196 (7k/5k deals)
  C blueprint vs 5×: chart6 +139±9 | loose +179±10 | tight +124±8 | rlcard +660±117 (400k/20k deals)
  1× ext vs 5×search-C: chart6 −53±135 | loose −98±157 | tight −61±124 (2k deals, wide).
  ⚠ Search underperforms the blueprint vs the chart rule bots by ~120 mbb (≈1.5σ each, consistent across 3)
  while beating it vs RLCard. Hypothesis: blueprint-likelihood beliefs mis-model off-blueprint opponents
  (chart bots' fixed opening ranges/sizes) → collapsed/wrong ranges. To investigate once the report lands.
  Live search bot 341 hands +85 bb/100 (±151). C 63.8% @25.53B; A(D) 82.9% @16.57B.
- ~18:20 local *** EXTERNAL GAUNTLET REPORT (sonnet agent, box B ~/ps4/logs/final): confirms interim table above;
  plus 1× rlcard vs 5×search −3142±513 (RLCard DQN RETRAINED on B — not the dev-box model; rows not comparable
  to old +587). Plumbing: bench::ext::parse_external + vsbp --hero/--field accept chart6*/rlcard:/ext: specs.
  MY GATE: cargo test -p bench -p search -p openpoker on dev → 82 passed, 0 failed.
  DIAGNOSIS started (B ~/ps4/run_diag.sh → logs/diag, all vs 5×chart6, seed 4104, 7k deals = same deals as
  search's +15.9±76.9): control `--no-search` == blueprint exactly (+96.9±68.0 on these deals; 400k-deal
  value +139) → harness OK; paired gap on identical deals ≈ −81. Arms running: bpbeliefs=0, flop=0, river-only.
- ~18:55 local: DIAGNOSIS (vs 5×chart6, identical deals seed 4104/7k, AIVAT): blueprint +96.9±68 | search default
  +15.9±77 | bpbeliefs=0 +42.9±76 | flop=0 (turn+river search) +59.4±70 | river-only (running).
  → monotone: the more streets searched, the worse vs chart6. Hypothesis: Bayes beliefs with an exact model
  likelihood zero out hands an off-model opponent really holds (rule bots take actions the model gives ~0).
  NEW KNOB `belief_tremble` ε (SearchParams; vsbp/openpoker `--set tremble=ε`): opponent likelihood
  (1-ε)·lik + ε/n in both the search-avg and blueprint Bayes paths; own seat exact; default 0 (no behaviour
  change). cargo test -p search -p openpoker on dev: 61 passed, 0 failed. Queued on B (~/ps5/run_tremble.sh →
  logs/tremble): ε=0.05 & 0.2 vs chart6 (seed 4104), ε=0.2 vs 5×C (seed 2027/30k; default +78.0±31.0).
  Live bot 421 hands +65 bb/100 adj (±123). C 65.2% @26.08B; A(D) 84.9% @16.97B (92% cap).
- ~19:25 local: vs 5×chart6 (seed 4104/7k): river-only search +46.6±66 (noflop +59.4, so not monotone — noise
  ~±40 on paired diffs); tremble ε=0.05 +16.0±79 (= default +15.9) → belief-collapse hypothesis NOT supported
  at ε=0.05 (ε=0.2 running; bpbeliefs=0 was +43). Working view: even river-only search gives up ~50 vs the
  blueprint here → likely a genuine "subgame-equilibrium vs blueprint-tuned" EV difference against a weak
  fixed opponent (search re-solves assuming blueprint-like ranges/continuations), not a harness bug.
- ~19:55 local: tremble ε=0.2 vs chart6 −17.2±81 (worse than ε=0 +15.9) → belief tremble does NOT fix it;
  hypothesis rejected; ε=0.2-vs-5×C arm cancelled. Knob kept (default 0, harmless). Next: turn+river-only search
  (flop=0) as a compromise — B ~/ps5/run_noflop.sh → logs/noflop: vs 5×C (seed 2027/30k) and vs chart6-loose
  (seed 4105/7k) with a same-seed blueprint control. Live bot 526 hands +59 bb/100 adj (±117).
  C 67.9% @27.17B; A(D) 88.2% @17.64B (92% cap). (Gotcha repeated: a pgrep/ps|grep pattern matched my own ssh
  bash -c line and killed the session — kill by pid/parent only.)
- ~20:25 local *** TURN+RIVER-ONLY SEARCH (flop=0) is the better final config:
  vs 5×C-final (seed 2027/30k): +87.4±26.5 (full search +78.0±31.0) → keeps the whole gain vs blueprint field;
  vs chart6-loose (seed 4105/7k, same deals): flop=0 +108.2±71 | blueprint +123.5±72 | full search +31.6±78;
  vs chart6 (seed 4104): flop=0 +59.4 | blueprint +96.9 | full search +15.9.
  → flop search is where the EV vs off-model opponents leaks; turn/river search ≈ blueprint vs rule bots and
  > blueprint vs blueprint-like play. Also cheaper (no flop solves).
  LIVE: Csearch session ended (577 hands, +95 bb/100 adj ±122, 0 fallbacks/restarts). Restarted the live bot
  with `--set flop=0` via scripts/op-search-loop2.sh → logs/openpoker-CsearchTR.jsonl.
  Confirmation arms on B (~/ps5/run_noflop2.sh): chart6-tight + RLCard, flop=0 vs blueprint, same seeds.
- ~20:58 local: confirmations (same seeds, flop=0 vs blueprint): chart6-tight +65.8±62.7 vs +124.9±61.2;
  RLCard(B-retrained) +709±214 vs +476±227. Summary in crates/search/RESULTS.md ("Final-bot config").
  FINAL CONFIG = C blueprint + turn/river search (flop=0). Probe robustness arms launched on B (~/ps5/logs/probe:
  probe bet=1.5 vs 5×{bp, search flop=0, search default}). Live TR bot: 49 hands, 0 fallbacks.
  C 70.8% @28.31B; A(D) 91.6% @18.32B (93% cap, ETA ~4 h).
- ~21:30 local *** LIVE BUG: DEEP STACKS. Open Poker cash stacks accumulate (no reset); our bot reached 22,098 chips
  = 1,105 bb and the engine is fed REAL stacks, so the 100bb-trained policy's "all-in" = a 1,105 bb preflop jam
  (hand 63, AKo 5-bet jam into JJ/AK/K5s, −22,098). Live TR session: 94 hands raw −1168 bb/100, all-in-adj −492;
  essentially all from that one hand. (Pluribus reset to 100 bb every hand — we didn't.) Bot busted → rebought
  to 100 bb, so the risk only recurs after it wins big. Sonnet agent launched: `--max-stack-bb X` (default 150):
  leave + rejoin at buy-in when stack > X·bb after a hand, plus a per-hand commit cap at X·bb as defence in depth;
  offline tests. Probe robustness (bet=1.5, C-final): vs 5×bp −7678±340 | vs 5×search flop=0 −7545±1232
  (≈ blueprint; flop search was what added the robustness, −10.9k on B) | vs default running.
  C 72.1% @28.84B; A(D) 93.2% @18.65B.
- ~22:15 local *** DEEP-STACK FIX DEPLOYED. Agent: `--max-stack-bb X` (default 150 live): (1) after a hand with
  stack > X·bb → leave_table → on own player_left → join_lobby at buy-in (abort+25-hand cooldown on refusal; idle
  rejoin retry); (2) per-hand commit cap at X·bb (raise/all-in beyond → raise to cap, else call/check; folds/calls
  untouched; traced `stack_guard`). 6 new session_replay tests. MY GATE: cargo test -p openpoker -p bench -p search
  on dev → 88 passed, 0 failed. Live: scripts/op-search-loop3.sh, binary target/opsearch-live2/openpoker, trace
  logs/openpoker-CsearchTR2.jsonl (stop: touch logs/openpoker-CsearchTR2.stop + kill pid). Previous TR session
  ended at ~100 hands. Mishap: `openpoker live --help` has no help → it ran a 2-hand builtin:tag session on our
  account (logs/openpoker-1792006405476.jsonl) before I killed it. Probe vs search default (C) still running on B.
- ~22:45 local: probe bet=1.5 vs 5×search DEFAULT (C-final): −10923±1275 (vs flop=0 −7545, blueprint −7678) →
  flop search buys off-menu robustness; flop=0 trades it for EV vs rule bots. Kept flop=0 for live (rule-bot-ish
  field); noted as the main config trade-off. Live TR2: 109 hands −15 bb/100 adj, 0 restarts. C 75.4% @30.18B;
  A(D) 97.2% @19.44B → finishes in ~1 h; then A-final eval (h2h vs B-final AND vs C-final).
- ~23:25 local: A(D) 99.4% @19.88B (94% cap, never store-full). Queued A-final pipeline (scratchpad afinal.sh,
  bg): wait for D's unit to exit → eval_checkpoint (gauntlet, h2h vs B-final 4M/format, 12M exploiter; --keep 3 so
  C-final compact used by the live bot is retained) → h2h A-final vs C-final 4M/format. Logs
  /tmp/claude-501/eval_Afinal.log, eval_Afinal_vsC.log. Live TR2: 179 hands +48 bb/100 adj (±117), stable.
- 2026-10-15 ~00:10 local *** A (paper sizing) TRAINING COMPLETE: 2e10 iters, 2130 min (35.5 h on D after resume),
  16.00B infosets, 40.3G slots (94% cap, store never full); checkpoint 628 s. A-final compaction running on D
  (9 arena groups) → eval pipeline queued. Box D then idle (delete candidate after the compact is on dev).
  STACK RESET VERIFIED LIVE: 2 resets (165 bb, 188 bb) → leave → player_left → join_lobby at 2000 → play resumed
  (261 hands TR2, +69 bb/100 adj ±99). C 79.4% @31.78B.
- ~00:56 local: A-final compact = 187 GB (C-final 125 GB), transferring D→dev (~25 min); eval follows. Note: dev
  (251 GB RAM) will mmap A-final 187 GB + B/C compacts + live bot's C-final 125 GB → page-cache pressure; watch the
  live bot's fallbacks. Live TR2: 333 hands +48 bb/100 adj (±91), 2 resets, 0 aborts. C 81.2% @32.48B.
- ~01:45 local *** A-FINAL (paper sizing, 2e10) eval: 6-max gauntlet random +3191 / station +2194 / nit +125 /
  tag +332 / lag +862 / maniac +1907 (all >0). h2h vs B-final 4M/format: 3A+3B −1.8±1.4 | 1A vs 5B −5.8±2.4 |
  1B vs 5A B −0.7±2.3 (trend −10.9 @4.0B → −3.8 @9.7B → −1.8 @20B). Ranking at 2e10: C (+1.6) > B > A (−1.8).
  Exploiter (12M) on A-final at 20%; h2h A-final vs C-final queued after it.
  *** INCIDENT: A's resume on D (iters 9.4e10) was OOM-KILLED at 23:34 UTC (anon RSS 589 GB > 566 GB) and
  systemd restarted it into a second OOM-bound load → stopped it. ROOT CAUSE blueprint::io::load sized the index
  with max_infosets.max(n×5/4): A's checkpoint holds 16.0B infosets (> configured 14.0B max, allowed because
  training fills the 10/7-sized table up to 85% load) → 20B → 28.6B-entry index = 457 GB (training: 320 GB) +
  180 GB arena. FIX: max_infosets.max(n×7/8+1) (grow only above 80% load at load time). Patched that single line
  on D and A (NOT `remote.sh sync`: its rsync --delete would delete the boxes' ~/pluribus/train.log /
  continue8d.log) and rebuilt; blueprint tests 7/7 on dev. C's future 4e10 resume on A gets the fixed binary
  (C n≈11.6B → would have needed ~331 GB index + 165 GB arena: close to the limit). A restarted 23:4x UTC;
  reload watched (peak RSS).
- ~02:15 local *** LOADER FIX VERIFIED: A resumed on D at 2e10 (15.996B infosets), peak RSS 470 GB (was 589 GB →
  OOM), steady 470.6 GB, ~215k it/s at start; free 110 GB. A now training toward 9.4e10 (8-day budget).
- ~02:25 local: A-final own-menu exploiter (12M): −372.1±60.7 → unexploitable (C-final −323). h2h A-final vs
  C-final (4M/format) started. A(D) 21.7% of 9.4e10 @20.42B, 217k it/s (faster than the 144k before: infoset
  set ~saturated → fewer allocations); C 85.1% @34.03B 284k it/s. Live TR2 483 hands +52 bb/100 adj (±84).
- ~02:50 local: wrote docs/TABLE_API.md (user will build a 6-max table API): REST (players/tables/matches/hand
  histories) + WebSocket live protocol (hand_start/action/street/your_turn/hand_end, decision_id idempotency,
  seq/resync), "to"-amount semantics, rules (min-raise, short all-in no reopen, side pots), stack_mode reset,
  deterministic seeded decks + 6-rotation duplicate, full-information hand_end for AIVAT, error codes,
  conformance vectors.
- ~02:50 local *** A-final vs C-final (4M deals/format): 3A+3C −4.0±1.3 | 1A vs 5C −5.7±2.2 | 1C vs 5A C +5.8±2.2
  → C clearly stronger than A at 2e10 (all three formats significant). Final ranking at 2e10: C > B > A.
- ~19:57 local: C@47.7B own-menu exploiter (12M): −289.9±59.1 → unexploitable (C@2e10 −322.9). h2h C@47.7B vs C@2e10
  running. C 30.2% @55.33B 358k it/s; A(D) 36.5% @34.31B. Live TR2 2,150 hands +72 bb/100 adj [21, 123].
- ~20:05 local: BAD RUN caught: h2h "C@47.7B vs C@2e10" gave −2364 mbb/g because the reused-tag invocation (no
  --run-dir) read sizing from box A's default run dir run-paper → the medium compact was played with the paper menu.
  Discarded; re-running with --run-dir run-medium --sizing medium. FIX: eval_checkpoint.sh writes $DDIR/sizing on
  every compact and a --no-compact/--tag run reads it back; backfilled sizing files for all 6 tags on dev.
  (A-final vs C-final earlier was correct: host D's default run dir is run-paper = A's.)
- ~20:30 local: re-run #1 died with a bash syntax error because I edited eval_checkpoint.sh WHILE it was executing (bash
  reads scripts incrementally) — not a script bug. Gotcha: never edit a shell script that is running; copy it first.
  Re-run #2 started (C@47.7B vs C@2e10, medium/medium, 4M deals/format).
- ~21:05 local *** C@47.7B vs C@2e10 (both medium, 4M deals/format, corrected run): 3+3 +3.8±1.0 | 1 new vs 5 old +5.3±1.6 |
  1 old vs 5 new old −6.8±1.6 → the extended training is clearly stronger than the 2e10 blueprint (all formats ≫ CI).
  Confirms: keep C training to 1.83e11; final bot = final C checkpoint (+ turn/river search).
- ~22:45 local: A-raw-C55 all 376 parts up; completion via finish_only.sh (box A deletion stays MANUAL after the SvanBot benchmark). D-raw-A34 435/528.
- ~23:45 local: SvanBot arm (c) search(flop=0) vs 5×Svan, 40k deals: +18.3±24.7 mbb/g (raw +29.4±30.1) — positive but not significant; blueprint alone got +42.4±7.7 (400k). Arm (d) Svan vs 5×search (12k deals) running.
- 2026-10-16 ~01:40 local: SvanBot agent died (weekly rate limit) after all 4 arms finished: (c) search vs 5×Svan +18.3±24.7, (d) Svan vs 5×search −30.3±43.6. VERIFIED bridge (~/svan/crates/apps/core/src/bin/extbridge.rs on box A, SvanBot commit 9a46bc6): real decide_with + learning ModelStore, history replayed through SvanBot's own engine; 25M decisions, 0 replay mismatches/errors/panics; selftest ok; VPIP 18–27/PFR 13–18. CAVEAT FOUND: bridge used Params::default() = the SIM budget (2,500 MC samples, 0.75 ms/decision) while SvanBot LIVE uses 640× (1.6M samples) + an NN/learned params from its DB → re-running (a)/(b) with SVAN_SAMPLES=1600000 (≈0.39 s/decision): a2 50k deals, b2 20k deals.
- 2026-10-16 ~02:07 local *** a2: SvanBot at LIVE budget (1.6M samples, 387 ms/decision, 387k decisions, 0 errors) vs 5×C@4.77e10 blueprint, 50k deals: AIVAT −46.9±21.2 mbb/g (raw −52.6±26.6) → still clearly loses (sim budget: −66.1±7.4). b2 (bp vs 5×Svan LIVE, 20k deals) running.