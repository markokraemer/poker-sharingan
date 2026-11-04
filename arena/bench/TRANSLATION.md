# Off-menu action translation: historical study and current tracking

The B-model measurements below describe the earlier shadow-game fix. They predate the
November 2026 tiny-bet exploit and `keep_sync` fix. Current defaults are
`BelowMenu::Smallest`, deterministic translation, `keep_sync = true`, and a strength-based
fallback. See [the current experiments](../../docs/RESULTS.md#action-translation-leak-and-fix-nov-2026),
including the model-input audit and independently reproduced frozen attack. Historical
`Check` results below do not justify restoring the old default.

Question: why does a trained exploiter whose menu is richer than the blueprint's ("paper" learner vs
blueprint B, sizing "local") win +26,224 +- 501 mbb/game, while the same-menu exploiter loses
-205 +- 57? Genuine translation exploit, bug in the target's off-menu handling, or evaluation artefact?

## Historical B-study result

**Mostly a bug in how the target policy handled off-menu actions (BlueprintPolicy/CompactPolicy
history tracking), not the pseudo-harmonic mapping itself. Not an evaluation artefact.** After the
fix the same exploiter wins +700 +- 149 (final B, 12M iterations), a 37x reduction. The remainder is
a residual attack against that historical configuration; its mechanism does not establish
the cause of every later attack.

* Not an artefact: raw (non-AIVAT) and AIVAT estimates agree (26,156 vs 26,224; 25,614 vs 25,625
  in the re-runs), the exploiter and target are the same objects in training and evaluation, and
  payoffs are zero-sum chip counts from the game engine.
* Root cause (bug): after the target observes an off-menu raise it appended the *translated index*
  to its abstract history, but kept computing its action list from the *real* state. The blueprint
  node reached by the translated history has a different action list than the real state (different
  raise depth menu, different clamping, a raise turned into a "check", an overbet turned into
  "all-in"), so
  1. the infoset lookup usually failed (`n_actions` mismatch) -> **fallback "check/call"**: after
     an off-menu action the target missed **92%** of its decisions and played fold 4.9% / call 2.8%
     / raise 0.6% of probability mass, i.e. it **never folds and never raises** - a calling
     station for the rest of the hand;
  2. when `n_actions` happened to match, index `i` of the *check node* was applied to the real
     `[Fold, Call, ...]` list (probability of "check" became probability of **fold**);
  3. an overbet translated to the abstract all-in always landed on a node with `[Fold, Call]`,
     100% miss, so every overbet >= 3x pot was called with everything.
  A learner with 14 preflop open sizes lives off-menu constantly (every size not in B's 4-size
  menu), so it found "value bet / shove into a station" in all positions: +51 bb/hand from UTG.
* Probes confirm (bench `probe:` bots, `research/experiments/probes.sh`, B-final, AIVAT, 8000 deals x 6 seats,
  probe's mbb/game; the always-call station baseline is -6704): legacy target `bet=0.1` -956,
  `bet=0.2` -1828, `bet=1.5` -16171... i.e. the target is wildly more or less exploitable by *size*
  alone, with miss rates 20-100% after the bet. After the fix `bet=0.1` -7317, `bet=0.2` -7582,
  `bet=3` -7438, `bet=5` -8560: all at or below the station baseline.

## Current tracking (updated 2026-11-04)

`nlhe::abstraction`:
* `Shadow`: the policy tracks a **shadow game** in which every off-menu raise is replaced by its
  translated abstract action. Action lists, history key and node identity come from the shadow, so
  the blueprint is always queried at a node of its own tree. The chosen abstract action is mapped
  back to a real action by pot fraction (`real_action`; abstract all-in = real all-in; clamped to
  the legal range). The policy's own actions are mapped back to the list entry they came from (no
  random re-translation). If the shadow cannot follow the real game (different actor/street, shadow
  terminal, unrepresentable fold), it is flagged desynced. With the current default
  `keep_sync`, the policy uses `bench::bp::fallback_action`, based on hand strength against
  random opponent hands and the actual call price. This fallback is heuristic, not range-aware.
* `keep_sync` checks each proposed translation one action ahead. It preserves terminal status,
  actor/street and every seat's folded/all-in status. If necessary, it chooses the nearest
  abstract action that stays in step. If none exists, such as a non-all-in raise past the tree's
  cap, it hands off explicitly with reason 4. It does not guarantee correct pot prices or ranges.
* `BelowMenu::{Check, Smallest}`: bets below the smallest abstract raise. `Check` = pseudo-harmonic
  between "check/call" (size 0) and the smallest raise. That historical rule could lose the real
  game after a tiny bet. `Smallest` = always the smallest raise, and **is the current default**.
  The old B-table comparison was made before the current synchronization and fallback fixes.
* `allin_snap` (default 0.5): a non-all-in raise maps to the abstract all-in only if it leaves
  at most 0.5 x pot behind; otherwise to the largest smaller size. (Mapping a 3x-pot bet to a
  10x-pot "all-in" made the shadow all-in while the real players still had chips.)
* `Translation::Random` (paper: randomised pseudo-harmonic when not searching): between two raise
  sizes only; "is it a bet at all" stays deterministic because a randomly ignored bet desynchronises
  the shadow (first attempt, random everywhere: +1,479 with 32% desync). Not AIVAT-exposable.
* `bench::bp::BlueprintPolicy` / `CompactPolicy` use `Shadow` (shared `bp::Source` code); exploit
  `abr.rs` tracks the target with the same `Shadow` so training and evaluation see the same
  target. `TrackOpts::PRE_SYNC_FIX` / `--pre-sync-fix` reproduce the tiny-bet-vulnerable
  configuration (shadow enabled, below-menu Check, no keep-sync). `TrackOpts::LEGACY` /
  `--legacy` reproduce the still earlier real-state-list tracking. Other diagnostic selectors:
  `--translation det|random`, `--below check|smallest`, `--allin-snap X`.
* Diagnostics: `PolicyFactory::report()`; bench prints per-policy decisions split by
  on-menu / after-off-menu: miss rate, desync rate (+reasons), fold/call/raise mass.
* Probes: `bench::probe` (`probe:pre=..,bet=..,raise=..,street=..`), `research/experiments/probes.sh`.
* Tests: `nlhe::abstraction::shadow_tests`, `bench/tests/harness.rs::off_menu_bets_keep_the_policy_in_its_tree`.

## Historical numbers (before the November keep-sync fix)

Exploiter = trained best response, "paper" learner menu, 1 learner seat vs 5 frozen target copies, all
seats, 20,000 eval deals, AIVAT (mbb/game, positive = target exploited).

Final B (2e10 it, sizing local), 12M exploiter iterations:

| target tracking | exploiter vs B |
|---|---|
| legacy (before) | **+26,007 +- 500** (seats 12708/7116/51413/38515/29004/17285), target miss after off-menu 91.7% |
| fixed: Check, allin_snap 0.5, deterministic | **+700 +- 149** (seats 1080/68/324/756/727/1244), miss 5.0% (all desync) |
| fixed + random between sizes | +619 +- 134 (seats 881/127/258/500/699/1248); within noise of deterministic |
| (reference) same-menu "local" learner, old B 12M | -205 +- 57 |

Ablation on the earlier B (4.5e9 it), 4M exploiter iterations (legacy +25,625):

| variant | exploiter |
|---|---|
| shadow, Smallest, snap 0 | +2,053 +- 236 |
| shadow, Smallest, snap 0, random | +1,944 +- 239 |
| shadow, Smallest, snap 0.5 | +2,141 +- 235 |
| shadow, Check, snap 0 | +256 +- 135 |
| **shadow, Check, snap 0.5 (then default)** | **+281 +- 136** |
| shadow, Check, snap 2 | +460 +- 129 |
| shadow, Check, snap 0.5, random everywhere | +1,479 +- 175 |
| shadow, Check, snap 0.5, random between sizes only | +151 +- 125 |

Against A (sizing paper, same-menu exploiter, 4M iterations; no off-menu actions exist so nothing
should change): legacy -452 +- 104, fixed -427 +- 104. (12M earlier: +470 +- 127.)

Probes vs B-final (probe's mbb/game; station baseline -6704; legacy -> fixed (default)):
`pre=8` -6257 -> -6990; `bet=0.1` -956 -> -7317; `bet=0.2` -1828 -> -7582; `bet=0.3` -5612 -> -5884;
`bet=0.75` -7625 -> -5513; `bet=1.5` -16171 -> -7462; `bet=2` -17011 -> -7593; `bet=3` -15440 -> -7438;
`bet=5` -14412 -> -8560; `bet=0.5,raise=3` -9001 -> -7505; `bet=3` river -8202 -> -8499.
Full tables: `research/experiments/probes.sh` (legacy, default, `--below smallest`), logs on the box.
Remaining relative gain of a probe over the station is <= ~1 bb/hand and similar to the on-menu
control (`bet=1`: -5954).

SearchPolicy (HU, tiny-menu 3M blueprint `data/search-bp/hu_tiny_3m.bin`, 2,000 search iterations,
3,000 deals, `sharingan-1/search/examples/probe.rs`; probe's mbb/game, lower = better for the agent):

| probe | bp legacy | bp fixed | search |
|---|---|---|---|
| station | -2434 | -2434 | -146 |
| bet 0.1 pot | -1125 | -2846 | -9723 |
| bet 0.3 | -1422 | -3600 | -9400 |
| bet 3 | -354 | -4679 | -13011 |
| bet 3, river | -1843 | -5877 | -9608 |
| pre 8bb | -694 | -4737 | -4749 |

Search (off-tree actions added to the subgame) beats every off-menu probe by far more than the
blueprint, even the fixed one; the old blueprint was the worst (+2 bb/hand over the station on
`bet=3`). Caveat: search is oddly weak vs the station here (-146), a property of that small test
setup, not of translation.

## Current limitations and historical caveats

* The current fixed C blueprint still admits a profitable frozen attack: the 1M policy has
  reproduced exactly at +419.5 ± 95.4 mbb/game on an independently verified public model.
  Stronger original-host figures remain subject to the input audit. Final-model confirmation
  and attack-specific causes are tracked in [RESULTS](../../docs/RESULTS.md#action-translation-leak-and-fix-nov-2026).
* The historical B study found +700 from a 12M learner. Random translation between raises
  gave +619 ± 134 versus +700 ± 149; that comparison did not establish an improvement.
  Neither result measures the present default configuration.
* In the historical Check/no-keep-sync configuration, tiny bets could desynchronize the
  shadow and trigger check/call. Current defaults avoid ignoring the tiny bet and use an
  explicit hand-off with a strength fallback when synchronization is impossible.
* Off-menu pot sizes still differ between the real and shadow games. Whether sizes near
  translation thresholds, overbets, or raise-cap hand-offs explain a particular attack must
  be measured. A later price-mass correction was reverted after an adaptive regression.
* Search now uses `search::bp::Blueprint` with `Shadow`, including the same tracking options
  and strength fallback. The earlier warning that search used legacy real-state histories
  is obsolete. Adversarial testing of the full search/API path remains separate work.
* Historical off-menu LBR/head-to-head measurements made with old tracking do not describe
  the fixed target. Use the command, model identity and tracking options recorded for each run.
* The scripted sizing probes are weak opponents. Their losses do not bound what a trained
  or adaptive attacker can earn. No six-player bot is proved unexploitable by these tests.
