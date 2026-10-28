# Off-menu action translation: the +26 bb/hand exploiter, root cause and fixes

Question: why does a trained exploiter whose menu is richer than the blueprint's ("paper" learner vs
blueprint B, sizing "local") win +26,224 +- 501 mbb/game, while the same-menu exploiter loses
-205 +- 57? Genuine translation exploit, bug in the target's off-menu handling, or evaluation artefact?

## Verdict

**Mostly a bug in how the target policy handled off-menu actions (BlueprintPolicy/CompactPolicy
history tracking), not the pseudo-harmonic mapping itself. Not an evaluation artefact.** After the
fix the same exploiter wins +700 +- 149 (final B, 12M iterations), a 37x reduction. The remainder is
the genuine translation weakness the paper itself describes (needs search).

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

## Fixes (all backwards compatible; `TrackOpts::LEGACY` / `--legacy` reproduce the old behaviour)

`nlhe::abstraction`:
* `Shadow`: the policy tracks a **shadow game** in which every off-menu raise is replaced by its
  translated abstract action. Action lists, history key and node identity come from the shadow, so
  the blueprint is always queried at a node of its own tree. The chosen abstract action is mapped
  back to a real action by pot fraction (`real_action`; abstract all-in = real all-in; clamped to
  the legal range). The policy's own actions are mapped back to the list entry they came from (no
  random re-translation). If the shadow cannot follow the real game (different actor/street, shadow
  terminal, unrepresentable fold) it is flagged desynced and the policy plays check/call.
* `BelowMenu::{Check, Smallest}`: bets below the smallest abstract raise. `Check` = pseudo-harmonic
  between "check/call" (size 0) and the smallest raise (the old rule, now consistent because the
  shadow really treats it as a check). `Smallest` = always the smallest raise. **`Check` is the
  default and the measured winner (see table): mapping tiny bets to the smallest raise lets the
  exploiter buy folds with min-bets (+2,000 mbb vs +260).**
* `allin_snap` (default 0.5): a non-all-in raise maps to the abstract all-in only if it leaves
  at most 0.5 x pot behind; otherwise to the largest smaller size. (Mapping a 3x-pot bet to a
  10x-pot "all-in" made the shadow all-in while the real players still had chips.)
* `Translation::Random` (paper: randomised pseudo-harmonic when not searching): between two raise
  sizes only; "is it a bet at all" stays deterministic because a randomly ignored bet desynchronises
  the shadow (first attempt, random everywhere: +1,479 with 32% desync). Not AIVAT-exposable.
* `bench::bp::BlueprintPolicy` / `CompactPolicy` use `Shadow` (shared `bp::Source` code); exploit
  `abr.rs` tracks the target with the same `Shadow` so training and evaluation see the same
  target. Flags: `--legacy`, `--translation det|random`, `--below check|smallest`, `--allin-snap X`
  (bench `match` and `pluribus exploit-compact`).
* Diagnostics: `PolicyFactory::report()`; bench prints per-policy decisions split by
  on-menu / after-off-menu: miss rate, desync rate (+reasons), fold/call/raise mass.
* Probes: `bench::probe` (`probe:pre=..,bet=..,raise=..,street=..`), `research/experiments/probes.sh`.
* Tests: `nlhe::abstraction::shadow_tests` (6), `bench/tests/harness.rs::off_menu_bets_keep_the_policy_in_its_tree`.

## Numbers

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
| **shadow, Check, snap 0.5 (default)** | **+281 +- 136** |
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

## Remaining risk

* The residual is real: +700 mbb vs B (7 bb/100) from a 12M-iteration learner that is still
  improving; richer learners / longer training will raise it. It is the paper's translation
  weakness (size-dependent folding/calling). Random-between-sizes helped only marginally (12M: +619 vs +700; 4M:
  +151 vs +281, inside noise).
* Check mode still desyncs after tiny bets (about 40% of the target's decisions after a bet below
  ~0.15 pot: the shadow treats it as a check, so players who already checked cannot be re-asked);
  it falls back to check/call there. Cheap to exploit only for small stakes, but not zero.
* Overbets > largest menu size are treated as the largest size (not as a shove); they are still
  the likeliest remaining leak (blueprint-only). Search is the proper remedy.
* `search` crate keeps its own blueprint history (`search::bp::Blueprint::translate`, real-state
  lists), i.e. the legacy tracking: off-tree handling inside the search tree still has the
  node-mismatch problem for earlier-round off-menu actions. Not changed here.
* `LBR`/`h2h` numbers computed earlier against off-menu opponents with the legacy tracking
  understate the blueprint's strength; re-run if they matter.
* Probes are fish (never fold); they only show relative effects. The trained exploiter is the
  real measurement.
