# SHARINGAN results

Everything measured so far, in one place. Conventions: 6-max no-limit hold'em, 100 bb, duplicate deals + AIVAT variance reduction,
mbb/game = thousandths of a big blind per hand (1 mbb/game = 0.1 bb/100), 95% confidence intervals. Blueprints: **A** = paper-style menu,
**B** = `local` menu, **C** = `medium` menu (final). See [EVALUATION.md](EVALUATION.md) for how each number is produced.

SHARINGAN-1 = C@2e10 + turn/river search. The inference API (`/v1/act`) plays it with search; the leaderboard's sharingan-1 track, `sharingan play --vs sharingan-1`
and the browser play page use the released C@2e10 blueprint without search. The search results below (+87 vs the blueprint, Open Poker live) are the search bot.
## Action-translation leak and fix (Nov 2026)

**2026-11-04 model-integrity audit:** numerical C evaluations are paused. Repeated
read-only reads of the provided C file returned different bytes at the same offset,
with unchanged inode, size, mtime and ctime. The public copy and one direct read agree;
other reads return zero pages in that key-index region. Earlier C estimates below
are historical original-host measurements; their scope remains provisional until a
verified independent copy reproduces them. The frozen 1M attack has since reproduced
exactly on the independent public C copy (+419.5 ± 95.4); the 3M and LBR figures still
await independent confirmation. No model, RAID configuration or live service
has been modified. See iteration 7 below. This is an input-integrity concern, separate
from the fixed translation bug.

**The leak.** A competition entry ("sombra", submitted 2026-11-03) scored **+5,231 ± 494 mbb/game** over a verified 20,004-hand run against five copies of the leaderboard blueprint (C@2e10, no search). It does not play better poker. It replicates our action translation from the public code and:

1. bets less than the smallest abstract size. With `BelowMenu::Check` (the old default) the blueprint read that bet as a check, which closed the street in its shadow game but not at the table;
2. once the shadow lost the real game, the policy fell back to check/call for the rest of the hand (`BpPlayer::dist`), with any cards;
3. checked to the river against these calling stations and shoved when ahead.

The caveat was documented below ("default `BelowMenu::Check` leaves ~desync after tiny bets ... `--below smallest` would avoid that but was not measured") and shipped anyway. The 12M-iteration exploiter above could not find it: its bets come from a menu with no tiny sizes, so it never triggers the desync. "No leak found" measured the wrong attack surface.

**The fix** (`engine/nlhe/src/abstraction.rs`, `arena/bench/src/bp.rs`, `sharingan-1/search/src/bp.rs`; `TrackOpts::PRE_SYNC_FIX` keeps the old behaviour for A/B):

* `BelowMenu::Smallest` by default: a bet is never read as a check.
* `keep_sync`: each translated action is checked one step ahead. If it would leave the shadow out of step with the real game (other player to act, other street, different all-in or folded seats, terminal mismatch), the nearest abstract action that keeps it in step is used instead. If none exists (a non-all-in raise past the tree's raise cap), the shadow hands off explicitly instead of tracking a phantom all-in.
* When the blueprint cannot be queried (hand-off, or a missing infoset), the policy plays by hand strength (Monte Carlo equity against the players still in, continuing only clearly above the price) instead of check/call.

Unit test `keep_sync_follows_adversarial_sizes`: one adversary with free sizes vs five followers, 6,000 hands: the shadow loses the real game in 2.8% of hands (all explicit hand-offs at the raise cap, zero silent drifts) vs 6.7% before, all silent.

**Measured** (local worker, 20,004 hands, duplicate + AIVAT, same deals for both columns; mbb/game for the attacking bot, 95% CI):

| attacker | vs C@2e10 before | vs C@2e10 fixed | vs SHARINGAN-2 before | vs SHARINGAN-2 fixed |
|---|---|---|---|---|
| sombra (the live exploit, its own code) | +5,382 ± 514 | **−420 ± 162** | +5,314 ± 502 | **−390 ± 156** |
| min-bet everything | −6,801 ± 478 | −5,983 ± 637 | −6,783 ± 479 | −5,933 ± 639 |
| min-raise every time | −3,181 ± 356 | −8,938 ± 736 | −3,199 ± 363 | −8,950 ± 739 |
| random sizes | −4,839 ± 405 | −5,745 ± 486 | −4,951 ± 404 | −5,651 ± 480 |
| tiny bets, shove the river | −1,207 ± 584 | −1,802 ± 353 | −1,370 ± 586 | −1,784 ± 352 |
| sizes at the translation midpoints | −4,520 ± 574 | −4,174 ± 573 | −4,538 ± 583 | −4,367 ± 585 |
| overbets (3x pot) | −682 ± 194 | −662 ± 197 | −752 ± 195 | −710 ± 198 |
| control: SDK tight-aggressive | −354 ± 118 | −466 ± 119 | −253 ± 116 | −382 ± 115 |
| control: SDK quickstart (old live-client file) | invalid (forfeits) | invalid (forfeits) | invalid (forfeits) | invalid (forfeits) |
| control: SDK quickstart (offline function, iteration 2) | −354 ± 100 | −596 ± 135 | pending | pending |

**Control correction (iteration 1).** The original quickstart results were almost entirely forfeits (19,758/20,004 on C@2e10; 19,749/20,004 on SHARINGAN-2; all eight lanes crashed before hello), so they do **not** prove unchanged on-menu play. They are withdrawn; a valid function-only C@2e10 rerun is listed separately, with final-model confirmation pending. The SDK quickstart's 3bb raise can be off-menu or clamp to an off-menu minimum; a strict on-menu control requires a separate fixture. Trained exploiter with an adversarial menu (`--learner-sizing adversarial`: tiny bets, near-min raises, midpoint sizes, overbets, one raise past the tree's cap), 1M iterations, 1 learner vs 5 copies of C@2e10, 20,000 evaluation deals per seat:

| target | exploiter wins (mbb/game) |
|---|---|
| C@2e10, translation before the fix (positive control) | **+4,552 ± 256** (it finds the leak) |
| C@2e10, fixed | **+162 ± 215** (not significant; upper end of the 95% CI ≈ +377) |

The original 20k-deal evaluation found the old leak at sombra's scale and no significant pooled profit after the fix. **Hardening iteration 1 below supersedes that finding: the original learner wins +293 ± 96, and a corrected training-target model wins +499 ± 94 mbb/game against fixed C@2e10 in larger evaluations. The saved 3M / 35-GB attacker in iteration 2 now wins +2,307 ± 75 over two independent-deal evaluations.** The original result bounded what this exploiter found in 1M iterations and 20k evaluation deals; it is not a proof of unexploitability (none exists for 6-max). After off-menu actions the fixed target still hands off to its fallback in 2.2% of decisions (raise wars past the tree's cap), an initial suspect that still requires attack-specific localisation.

### Hardening iteration 1 — training/evaluation fallback mismatch (2026-11-04, kept)

**Hypothesis.** The adversarial learner understates attacks on the fixed bot because its
training target still plays check/call after a shadow hand-off or missing infoset, whereas
its evaluation opponent uses `bench::bp::fallback_action`. Longer training against that
surrogate need not improve attacks against the actual fixed bot. The saved 1M result also
filled its 15 GB learner store; seat-specific positives are exploratory, not a new pooled
profit claim.

**Candidate (uncommitted; no playing-policy change).** Use the evaluation policy's exact
strength-based fallback in `Exploiter::target_real_action`,
including actual private cards and only the currently visible board, whenever `keep_sync`
is enabled. Preserve check/call for the pre-fix positive control. Regression fixtures compare
training actions with `BlueprintPolicy` actions on missing infosets and explicit reason-4
hand-offs. Include reason 4 in the diagnostic `why` array (now reasons 1 through 5 in order).

**Worker and commands.** All compute is under `nice -n 10` on the existing worker, in
`~/fix`; the live services and model files are untouched. Preserve the pre-change executable
as `~/fix/hardening-01/exploit-before`. Launch jobs with `setsid nohup`, logs and JSON in
`~/fix/hardening-01/`. Baseline command from `~/fix`:

```sh
nice -n 10 ./hardening-01/exploit-before --compact \
  --target /srv/sharingan/models/sharingan-1.plcp --sizing medium \
  --buckets /srv/sharingan/models/buckets-200 --players 6 \
  --iters 1000000 --threads 11 --mem-gb 15 --learner-sizing adversarial \
  --eval-deals 100000 --seed 1 --out hardening-01/before.json
```

Required regression command, from `~/fix/sharingan` (8 build workers / 8 Rayon threads,
concurrent with the 11-thread baseline):

```sh
nice -n 10 env RAYON_NUM_THREADS=8 CARGO_BUILD_JOBS=8 \
  /home/ubuntu/.cargo/bin/cargo test --release -j 8 \
  -p nlhe -p bench -p search -p sharingan-arena -p exploit -- --test-threads=8
```

The baseline filled the learner store by 250k iterations (256,866,628 infosets,
1.87G slots), well before the 1M budget. This is an observed limitation, not a reason to
increase the memory in only one arm of the initial comparison.

Queued behind the release suite: rerun the new parity tests with `cargo test --release
-j 8 -p exploit --lib -- --test-threads=1 --nocapture`, build the requested five packages,
then run the candidate with the baseline flags above (output `after.json`). A 10k-iteration,
1 GB, 20k-deal pilot runs first. A candidate-binary pre-fix positive control follows
(`--pre-sync-fix`, 1M iterations, 15 GB, 20k evaluation deals); the queue stops if that
control's lower 95% bound is below +1,000 mbb/game. All stages propagate command failures.
The queue uses 11 learner workers and at most 8 build workers alongside the baseline;
it does not start until the existing release tests exit. Logs include `parity.log`,
`build.log`, `pilot.log`, `after.log`, `positive-control.log`, and `next.log`.

**Tests.** The required five-package release suite passed: 142 tests across 27 suites,
zero failed or ignored. The subsequent focused tests passed too, including the diagnostic
assertion. In 16 missing-infoset fixtures and 16 forced raise-cap fixtures, the old
check/call surrogate disagrees with the evaluated fixed target in 28/32 cases (16/16
missing, 12/16 hand-off); the corrected training target disagrees in 0/32. These are
deterministic implementation fixtures, not independent sampled games; no poker confidence
interval or real-game mismatch frequency can be inferred from them. Pre-fix target parity
is preserved in all 16 fixtures.
All 16 explicit hand-offs appear under reason 4 in the report.

**Pilot (plumbing only).** Candidate 10k iterations / 1 GB / 20k evaluation deals per
seat: attacker −5,344.6 ± 201.5 mbb/game; training 42.4 seconds; store full; learner
miss rate 46.8%. This deliberately undersized pilot is not evidence of robustness or
an A/B improvement. The corrected report attributes every observed desync to reason 4
(385 on-menu-classified decisions and 673 after-off-menu decisions); no silent drift
reason is reported. The matched 1M candidate run started after this pilot.

**Additional validation queued.** Once the baseline process exits successfully, rerun
`bash ab.sh sombra/bot.py h01-sombra`, then `bash ab.sh "$bot" "h01-$tag"` for all
`probes/probe_*.py`, `ctl_quickstart.py`, and `ctl_tag.py`. This uses the same
`ab-20004` deal seed and 20,004 hands per arm as the original results, eight lanes,
on C@2e10 only while the candidate learner runs on that model. The before/fixed
playing policies are unchanged by this harness candidate; these reruns check reproducibility
and the old-leak positive control. Output prefix `res-h01-`, log
`hardening-01/probes.log`. SHARINGAN-2 jobs must wait until all C@2e10 jobs finish.

**Probe/control rerun audit.** Sombra's pre-fix positive control reproduced +5,381.8 ±
513.9; fixed −419.6 ± 161.6. All six probe results and
tag reproduce the original table at the displayed precision; their fixed-arm JSON reports
zero forfeits, crashed lanes, and timeouts. The old quickstart file is a live-client example
with registration/join calls at import time, not an offline bot. It must not run again in
these experiments. The discarded-stderr harness logs do not establish whether its external
requests succeeded; do not query or change the live database to investigate. The new
function-only `arena/exploit/bots/ctl_quickstart.py` comes from the SDK README, contains no
client setup, and is queued under tag `h01-quickstart-offline` after the archive test job
frees worker capacity. The queue validates zero forfeits/crashes/timeouts and positive
decision count in both arms before treating it as a control.

**Next isolated hypothesis.** After the learner store fills, `traverse` samples uniform
unseen learner actions while `ExploiterPlayer::dist` uses check/call. This second
training/evaluation mismatch is present in both current arms; do not fold its correction
into this A/B. Test it separately after the target-fallback comparison.

**Baseline evaluation completed.** Unmodified harness, 1M training iterations, 15 GB,
100,000 evaluation deals per seat: attacker **+293.3 ± 95.7 mbb/game** (raw +268.7 ±
115.0), a significant pooled profit against fixed C@2e10. Training took 1,386.5 seconds;
learner miss rate 10.65%, store full. This reproduces the earlier +161.7 ± 214.6 estimate
within its uncertainty and resolves a positive result with more evaluation deals. Parallel
training is not bit-reproducible despite per-iteration RNG seeds; this is a fresh training
run, not a re-evaluation of preserved old weights.

| zero-based learner seat | baseline attacker mbb/game, 95% CI |
|---|---|
| 0 | +21.5 ± 208.8 |
| 1 | +797.6 ± 227.2 |
| 2 | +797.5 ± 169.8 |
| 3 | +578.6 ± 161.2 |
| 4 | +72.6 ± 157.6 |
| 5 | −507.8 ± 161.7 |

The pooled result was evaluated on the fixed bot, even though its learner trained against
the mismatched fallback. It establishes an attack result, not its mechanism; the hand-off
hypothesis still needs a causal A/B. The completed positive-control rerun below reproduces the old leak. The seed overlaps
the earlier exploratory evaluation; independent-seed confirmation and final SHARINGAN-2
confirmation remain open. Executable SHA-256: baseline
`3c7739e665cfb1ea6632bd71bf11fd7239d45c33d9e4fc79f1a127da75fb8385`, candidate
`7fc70a6406238b15c012f599021dd92d4ee17cf8a4a78641f6eb5e3c4bea789f`.

**Corrected target-model result.** Same 1M training budget, 15 GB, training/deal seed,
all seats, and 100,000 evaluation deals per seat: attacker **+499.4 ± 94.0 mbb/game**
(raw +527.5 ± 112.7). The pooled 95% intervals do not overlap: baseline [197.6, 389.0],
corrected [405.4, 593.4]. This is a measured improvement in attack strength from training
against the actual fixed fallback; it does not strengthen the defending poker policy.
Training took 3,245.8 seconds, versus baseline 1,386.5; learner store full at 273,120,186
infosets, miss rate 10.44%, 7,312,456,064 traversal nodes. The evaluation reports only
explicit reason-4 hand-offs (2,821 on-menu-classified and 6,051 after-off-menu decisions),
with a 0.96% after-off-menu hand-off fraction under this attack. A lower hand-off fraction
does not imply lower exploitation: this attack wins more.

| zero-based learner seat | corrected attacker mbb/game, 95% CI |
|---|---|
| 0 | +259.2 ± 208.1 |
| 1 | +886.8 ± 224.8 |
| 2 | +848.4 ± 165.3 |
| 3 | +722.4 ± 157.5 |
| 4 | +297.6 ± 155.8 |
| 5 | −18.0 ± 157.4 |

**Positive control completed.** Candidate executable, pre-fix target, 1M iterations / 15 GB /
20k evaluation deals per seat: attacker **+4,421.5 ± 254.9 mbb/game** (raw +4,379.5 ±
295.8); training 1,738.7 seconds, store full. Its interval [4,166.5, 4,676.4] overlaps the
original +4,552.1 ± 255.8 interval [4,296.3, 4,807.9]. Both find the old leak at the
expected scale. The independent sombra rerun also reproduces the old leak and loses
against the fixed target, as recorded above.

**Decision: kept.** Target-model correction qualifies on the prespecified attack-strength
A/B, the positive controls reproduce, and the final reverted-source validation passed
143 tests and all five requested package builds. Archive infrastructure has exact
round-trip test evidence; the failed cache optimization is excluded. No defending-policy EV improvement is
claimed; independent deal seeds, final SHARINGAN-2 confirmation, causal localisation, and
any defending-policy fix remain open. The validated harness changes were committed and pushed to main as `1c51b24`. The defending decision logic and model
weights are unchanged (the sole `bench::bp` change is diagnostic serialization), so this
iteration does not claim a new head-to-head strength result. The 4M-deal SHARINGAN-2 vs
C@2e10 measurement is prior evidence, not a rerun for this harness-only change; an actual
defending-policy change must still pass the requested normal-strength A/B.

### Hardening iteration 2 — freeze the attacker and expand training (2026-11-04, in progress)

**Hypothesis.** A stronger lower-bound attack needs a larger learner store and 3M+
iterations; causal policy comparisons also need the *same learned strategy* in each arm.
The CLI previously discarded the learner on exit, preventing later fixed-attacker A/B
or final-model confirmation without another stochastic training run.

**Candidate infrastructure (no playing-policy change).** `--save-learner FILE` stores native
PLBP4 regrets/average counters without quantising probabilities, plus a `.json` manifest
recording players, learner menu, bucket configuration and training provenance.
`--load-learner FILE` skips training and restores the exact learner policy. Incompatible
player counts, menus, bucket paths or refinements are rejected before model loading.
Target model and target translation options may change for A/B; keep extraction mode fixed
when comparing the same attack. `--eval-seed` separates evaluation deals from training.
Archives require a fresh path and do not overwrite an existing archive. The original
candidate executable and iteration-1 jobs remain unchanged.

**Tests.** The five-package release suite passed again: **143 tests across 27 suites,
zero failed or ignored**. The final focused library run passed all three tests too.
A CLI round-trip fixture trains a small learner, saves it, reloads it with a
nonzero requested iteration budget, and requires identical per-seat/pooled AIVAT and raw
estimates and miss rates, zero new training iterations/nodes, and unchanged training
provenance. It also exercises rejection of four incompatible abstraction configurations.
The full suite used one test thread and eight Rayon/build workers. These checks prove
archive fidelity on the fixture, not poker strength.

**Validation gate and training started.** The repaired function-only quickstart completed
20,004 hands per arm with zero forfeits, crashed lanes or timeouts: pre-fix −354.4 ± 99.5,
fixed −596.4 ± 135.2 mbb/game (attacker perspective). This is a valid SDK-bot control,
not a strictly on-menu control; the fixed target wins more against it. Following the full
suite, build and final focused tests, the **3M / 35 GB run started**, all six seats,
training seed 2. First progress: 4,096 iterations in 33 seconds, store not full.
Commands from `~/fix`, inherited `nice -n 10`:

```sh
./hardening-02/exploit-archive --compact \
  --target /srv/sharingan/models/sharingan-1.plcp --sizing medium \
  --buckets /srv/sharingan/models/buckets-200 --players 6 \
  --iters 3000000 --threads 11 --mem-gb 35 --learner-sizing adversarial \
  --eval-deals 100000 --seed 2 --eval-seed 271828 \
  --save-learner hardening-02/learner-3m.bin --out hardening-02/train-3m.json
./hardening-02/exploit-archive --compact \
  --target /srv/sharingan/models/sharingan-1.plcp --sizing medium \
  --buckets /srv/sharingan/models/buckets-200 --players 6 --threads 11 \
  --learner-sizing adversarial --eval-deals 100000 --seed 2 --eval-seed 314159 \
  --load-learner hardening-02/learner-3m.bin --out hardening-02/holdout-3m.json
```

**Frozen 1M confirmation completed.** Training seed 1, 1M iterations / 15 GB, new evaluation
seed 271829, 100k deals per seat: attacker **+419.5 ± 95.4 mbb/game**, raw +467.0 ± 114.1.
Training took 3,438.7 seconds; 273,057,930 infosets, 1,875,000,000 slots, store full,
10.32% evaluation miss rate. The native archive is about 12 GB. Loading it with training
skipped reproduces **every pooled/per-seat/raw estimate exactly**, including uncertainty
and sample counts. This is model-scale archive fidelity evidence, beyond the toy fixture.

The *same frozen strategy* on independent seed 314160 (100k deals per seat) wins
**+495.3 ± 95.4**, raw +466.2 ± 114.0. Pool only these two evaluations of the same archive:
**+457.4 ± 67.4 mbb/game**, raw +466.6 ± 80.6, 200,000 independent duplicate-deal clusters
per seat (1.2M actual hands). The pooled variance includes both within-run sample variance
and between-run mean terms; artifact `hardening-02/pooled-1m-confirm.json`. Do not pool
the earlier +499 policy, which came from a different stochastic training run. These fresh
deal results corroborate profitable exploitation; they do not measure variance over
independent training seeds. Final SHARINGAN-2 confirmation remains pending.

**Initial standard LBR.** Seed 3 / 200 deals: strong **+688.4 ± 1,547.8**, weak **+239.5 ±
1,565.7 mbb/game**. Both are inconclusive, not evidence of profitable LBR. Ignore the
one-deal untrained-ABR placeholder and its meaningless zero sample-variance interval in
that JSON. A 20,000-deal strong/weak LBR run on fresh seed 5 is now live with eight
workers, at nice priority 10, alongside the 11-worker 3M run (and, briefly, the one-worker
audit). Command: the earlier LBR command with `--seed 5 --lbr-deals 20000`; wrapped in
`/usr/bin/time -p`. Output `hardening-05/lbr-20k.json`, log `hardening-05/lbr-20k.log`,
immutable driver `hardening-02/lbr-20k.sh` (PID 685702). Completed; see iteration 5 below.

**Storage observation.** By 894,576 iterations (29.8% of 3M), the 35 GB learner store
was full at 671,173,511 infosets and approximately 4.38G slots. The larger budget delays
saturation but does not remove it; remaining training still updates visited infosets, with
the previously documented unvisited-learner fallback mismatch. Raw elapsed times include
the recorded test pauses and are not a clean speed comparison.

**3M attacker and independent-deal confirmation completed.** The saved 3M / 35-GB
policy wins **+2,325.2 ± 106.5 mbb/game** on seed 271828, raw +2,387.2 ± 125.1.
The *same frozen weights*, without further training, win **+2,288.3 ± 106.2** on seed
314159, raw +2,331.5 ± 124.3. Each evaluation has 100,000 independent duplicate-deal
clusters per seat, 600,000 actual hands. Pool only these two runs of this archive:
**+2,306.7 ± 75.2**, raw **+2,359.3 ± 88.2**, 200,000 deal clusters/seat and 1.2M hands.
Every seat is significantly profitable; no post-hoc seat selection is needed.

| zero-based learner seat | pooled frozen 3M attack, mbb/game (95% CI) |
|---|---|
| 0 | +2,717.5 ± 153.6 |
| 1 | +3,356.5 ± 162.0 |
| 2 | +2,586.5 ± 134.0 |
| 3 | +2,126.4 ± 125.4 |
| 4 | +1,730.6 ± 119.9 |
| 5 | +1,322.8 ± 120.1 |

Training reports 24,855,210,370 traversal nodes, 671,173,511 infosets and
4,375,000,000 used slots; store full, evaluation miss rate 4.608%. Reported 15,093.8
training seconds include the earlier documented pauses, so do not use that elapsed time
as a clean performance comparison. This changes training seed, iteration budget and
memory versus the 1M study: it is a stronger measured attack, not an isolated causal
claim about which training change supplied the gain. Do not pool different learned
policies or treat fresh deal seeds as independent training replications.

A disabled experimental build reloaded `learner-3m.bin` and reproduced **all pooled,
per-seat and raw estimates, uncertainties, sample counts and learner miss rate exactly**
on seed 271828 (`hardening-06/3m-replay-base.json`). Its price correction is explicitly
false. This extends model-scale persistence validation to the larger archive. Exact
reload command from `~/fix`, inherited nice 10:

```sh
./hardening-06/exploit-price --compact \
  --target /srv/sharingan/models/sharingan-1.plcp --sizing medium \
  --buckets /srv/sharingan/models/buckets-200 --players 6 --threads 11 \
  --learner-sizing adversarial --load-learner hardening-02/learner-3m.bin \
  --seed 2 --eval-deals 100000 --eval-seed 271828 \
  --out hardening-06/3m-replay-base.json
```

**Decision: retain the measured attack bound and frozen weights.** No defending-policy
improvement is claimed by this training experiment. Logs and artifacts stay in
`~/fix/hardening-02/`; immutable queue `hardening-01/archive-train-v2.sh` propagated
failures and kept C@2e10 throughout. Final SHARINGAN-2 confirmation and mechanism
localisation remain open. This run retains the separate store-full learner-fallback
mismatch noted in iteration 1; fix and measure that independently.

**Earlier frozen confirmation queued.** To avoid waiting for the longer 3M run before
causal defending-policy tests, queue another faithful 1M / 15 GB learner after the new
pre-fix positive control, while the 3M run continues. Use the same training seed 1 but
fresh evaluation seed 271829; save `hardening-02/learner-1m-confirm.bin`. This is a fresh
stochastic training run, not restoration of the discarded +499 policy weights. Reload
that archive on the same 100k-deal evaluation and require exact pooled/per-seat/raw
estimates before trusting persistence at model scale. Then evaluate the same frozen
strategy with seed 314160, 100k deals per seat. Commands are the 3M/archive commands above
with `--iters 1000000 --mem-gb 15 --seed 1 --eval-seed 271829`, corresponding `1m-confirm`
paths, then `--load-learner` for replay and holdout. Independent *training*-seed confirmation
still belongs to the seed-2 3M run. Immutable queue `hardening-02/frozen-confirm-v2.sh`
(PID 640691) propagates failures and checks the new positive control plus reverted-source
validation before starting. It also requires the new positive-control 95% interval to
overlap the original +4,552 ± 256 result; a weaker unrelated positive profit is not enough.
The initial LBR job follows these evaluations in that queue;
it replaces the previously waiting `lbr-queued-v4.sh` job. No target-model switch or
extra concurrent training workers are introduced.

### Hardening iteration 3 — per-deal fallback equity cache (2026-11-04, reverted)

**Hypothesis / candidate.** Correctly modelling the fixed fallback is expensive because
counterfactual branches repeatedly compute deterministic 400-sample equity. Factor card
equity from the price-dependent decision, and cache it locally to one sampled deal by
acting seat, street and live-opponent count. Existing training binaries remain unchanged.

**Commands and scheduling.** Use `hardening-02/cache-now.sh` under `nice -n 10`.
Temporarily SIGSTOP only experimental 3M PID 602317 (last pre-suspension progress
174,576 iterations / 1,008 seconds), retaining its state; the corrected 1M run continues.
An EXIT trap SIGCONT's the experimental process on failure or completion. Run the required
five-package release suite with eight build/Rayon workers and one test thread, the focused
`cargo test --release -j 8 -p exploit --lib cached_fallback -- --test-threads=1 --nocapture`,
then build `hardening-03/exploit-cached`. The suite passed **144 tests / 27 suites**, with
zero failed or ignored. The focused fixture compared 231 states across 2–6 players and
all streets/opponent counts. It passed but did not establish end-to-end equivalence.

Alternate old/new executable order for seeds 11–15 using identical flags:

```sh
EXE --compact --target /srv/sharingan/models/sharingan-1.plcp --sizing medium \
  --buckets /srv/sharingan/models/buckets-200 --players 6 --iters 1000 \
  --threads 1 --mem-gb 1 --learner-sizing adversarial --eval-deals 200 --seed SEED
```

Require exact evaluation estimates and learner infoset/slot/node counts, miss rate and
store-full status. **The first seed failed this gate.** Seed 11: original 9,238,698 nodes /
2,251,181 infosets / 58.64 seconds; candidate 9,240,789 nodes / 2,251,423 infosets / 28.31
seconds. Pooled evaluation −4,326.0 ± 1,894.2 vs −4,379.8 ± 1,895.9. These are tiny
training/evaluation smoke runs, not useful attack bounds; the apparent speed ratio is
**discarded** because behaviour differed. Stop the benchmark parent and its active child;
do not complete the planned five-run timing analysis or infer a performance improvement.

**Investigation.** `hardening-03/investigate.sh` repeats each executable on the same
seed/flags. The original's evaluation and training counts repeat exactly; the candidate's
do not. The root cause is unresolved. Unit-level equality was insufficient and does not
overrule the end-to-end failure. The experimental 3M process resumed, then was suspended
again solely to validate the reverted source within the CPU budget; all suspensions are
preserved in its raw elapsed time and make its logged aggregate it/s unsuitable for speed
comparisons.

**Decision: reverted.** Remove the per-deal cache and fallback factoring in full; retain
only iteration 1's diagnostic reason-4 fix in `bench::bp`. Keep failed logs/JSON under
`~/fix/hardening-03/`. `hardening-03/reverted-tests.sh` (PID 627043) reruns the required
release suite and all five requested package builds on the reverted source, with an EXIT
trap resuming PID 602317. The reverted suite passed **143 tests / 27 suites**, zero failed
or ignored; all five requested package builds passed too. The EXIT trap resumed PID
602317, confirmed running afterward. No cache change is retained or committed.

**Standard LBR queued separately.** After the new iteration-1 positive control and reverted
source validation finish, run the original archive executable on C@2e10 with `--iters 0
--threads 8 --mem-gb 0.001 --learner-sizing adversarial --eval-deals 1 --seed 3
--lbr-deals 200`. Inspect uncertainty before scaling; the one-deal ABR placeholder is not
the LBR result. Output `hardening-02/lbr-200.json`, now sequenced after frozen 1M confirmation and
holdout in `hardening-02/frozen-confirm.sh` (PID 639829). Model jobs remain sequential across target
models; these jobs use C@2e10 only.

### Hardening iteration 4 — localise frozen-attacker profit (2026-11-04, kept)

**Hypothesis.** The remaining profitable attack may primarily exploit shadow hand-offs,
or may profit elsewhere in translation/blueprint play. The corrected learner's larger
profit and smaller hand-off frequency do not decide this. Attribute observational raw
payoffs by first actual target decision with an out-of-sync shadow, its street, and attacker
seat, before choosing a causal defending-policy A/B.

**Candidate (diagnostic only).** Optional `--audit-handoffs` wraps the sole attacker and
replays completed public histories through one shadow for each target, preserving each
seat's own-action translation. Only a target's actual turn with nonzero `sync_reason`
counts. Aggregate one record per hand, grouped by the first such street and attacker seat;
also report decision counts by street/reason. No cards or individual hand records are
exported. The diagnostic does not detect missing-infoset-only fallback, and rejects hidden
random translation. Conditional raw payoff means are explicitly **observational**, not
causal effects, AIVAT estimates, or independent-deal confidence intervals.

**Tests / resource gate.** Two new tests pass: exact match-estimate parity and raw-payoff
reconciliation (one record per actual hand, not per target/fork), with both passive and
forced raise-war fixtures; and rejection of random translation. The full required release suite passed **145 tests / 27 suites**, zero failed or ignored;
all five requested packages built successfully, under `nice -n 10`, eight build/Rayon
workers and one test thread. Only the 3M experiment (PID 602317) is temporarily suspended; the saved
1M confirmation keeps training on its immutable binary. Validation script `hardening-02/audit-tests.sh` (PID 651250) completed, and its EXIT
trap resumed the 3M process, confirmed running afterward.

**Model-scale measurement queued.** After validation and the existing frozen 1M confirmation,
replay, holdout and initial LBR sequence finish, run from `~/fix`:

```sh
./hardening-04/exploit-audit --compact \
  --target /srv/sharingan/models/sharingan-1.plcp --sizing medium \
  --buckets /srv/sharingan/models/buckets-200 --players 6 --threads 1 \
  --learner-sizing adversarial --eval-deals 100000 --seed 1 --eval-seed 271829 \
  --load-learner hardening-02/learner-1m-confirm.bin --audit-handoffs \
  --out hardening-04/audit-1m.json
```

The immutable `hardening-02/audit-run-v3.sh` queue (PID 658249) waits for validation,
then observes either the actual 8-worker LBR child (which follows the holdout) or completion
of the confirmation sequence. It checks validation/replay success markers and holdout
output, then runs at nice priority 10. Require *exact* overall/per-seat/raw evaluation
equality with the unwrapped frozen reference, 600,000 recorded hands, reconciliation of
raw payoff sums, and exact agreement of reconstructed reason counts with the target's own
diagnostic counters. Failure stops attribution. One audit worker + 8 LBR workers + 11
3M learner workers stays within the CPU budget; no target-model switch.
**Model-scale verification passed.** Exact evaluation parity, 600,000 recorded hands,
raw-payoff reconciliation and *all* target reason-count reconciliations pass. The audit
counts 8,895 shadow-fallback decisions, all explicit reason 4. Output
`hardening-04/audit-verified.json` is the validated aggregate, with no individual histories
or cards exported. Discovery evaluation seed 271829 is used for attribution; reserve
314160 for defending-policy confirmation after choosing the candidate.

| first actual shadow-fallback decision | hands | conditional raw mbb/game | contribution to overall raw mbb/game |
|---|---:|---:|---:|
| preflop | 2,381 | −13,054.6 | −51.8 |
| flop | 75 | −19,482.5 | −2.4 |
| turn | 383 | +15,889.0 | +10.1 |
| river | 103 | +25,047.7 | +4.3 |
| no shadow-fallback decision | 597,058 | +509.3 | +506.8 |

Only 2,942/600,000 hands (0.4903%) expose a target shadow fallback. Those hands contribute
**−39.8** raw mbb/game in aggregate; the no-handoff group contributes **+506.8**, summing
to **+467.0** raw overall. These conditional groups have no claimed confidence intervals
or causal interpretation. They do not prove raise-cap handling is safe, and they exclude
missing-infoset-only fallback. They show this frozen attack's net profit is overwhelmingly
in hands without a shadow hand-off, rather than in the originally suspected fallback.

**Decision: keep the opt-in diagnostic**, supported by unchanged decisions/estimates and
exact target-counter reconciliation. No defending-policy improvement is claimed. Next
hypothesis: residual off-menu translation distorts actual versus shadow call prices.
Test a parameter-free, opt-in correction of fold/call probability mass for price mismatch,
leaving raise mass and equal-price/on-menu decisions unchanged. This is a hypothesis for
a causal A/B, not an attributed cause or an accepted fix.

**Additional test capacity (2026-11-04).** Read-only Platinum inventory/context checks
found an already-running 16-CPU / 64-GB primary with headroom and the same Rust 1.99.1
compiler. No machine was created or resized. To reduce future pauses of the 3M run,
prepare a separate `/work/sharingan-hardening-1c51-audit` source snapshot for model-free
release tests. Copy only Rust sources, Cargo manifests/lockfile, schema migrations and
root notices/readme (190 files, 512,079-byte archive); no `.git`, environment files,
credentials, model files, website, or `docs/research/`. Archive SHA-256:
`6a2c0bb63c4eeb41e597cfd96be076170431c3f6b6c950a95fe44e5021ec3180`.

Validation runs asynchronously (launcher PID 1863862; heavy child processes confirmed
nice 10 and affinity CPUs 0–3). Use `taskset -c 0-3`, `env -i`, a dedicated Cargo cache,
target directory and TMPDIR, `RAYON_NUM_THREADS=4`, `CARGO_BUILD_JOBS=4`, the same required
five-package `cargo test --release -j 4 ... -- --test-threads=1`, then the requested
five-package release build. Logs: `/work/sharingan-hardening-1c51-audit/validation.log`;
success marker `validation.ok`. Validation completed: **145 tests / 27 suites**, zero
failed or ignored, and all five requested package builds passed. The source-only test
worker is now available for future checks without suspending model training.
Training remains on the original worker within its 22-worker budget. Runtime binaries for
that worker continue to be built there: the Platinum image has glibc 2.43 and the arena
worker glibc 2.39, so cross-host binary portability is not assumed. This adds test capacity
without changing live services or the training jobs.

### Hardening iteration 5 — standard local best response (2026-11-04, measured)

**Hypothesis.** A range-tracking local best response can expose a profitable deviation
that the finite-store adversarial learner misses. No defending-policy change in this
iteration. Run the standard strong and weak LBR configurations against fixed C@2e10,
20,000 duplicate deals with the attacker in all six seats (120,000 hands per configuration).
The implementation measures actual chip payoffs, averages the six seats within each deal,
and forms its 95% CI over those 20,000 independent deal clusters. These are raw winnings,
without AIVAT, against a five-copy policy profile; this is not a six-player equilibrium-gap
bound. Strong and weak configurations use the same deal seed, so do not pool them.

Exact command from `~/fix`, inherited nice 10, eight Rayon workers:

```sh
/usr/bin/time -p ./hardening-02/exploit-archive --compact \
  --target /srv/sharingan/models/sharingan-1.plcp --sizing medium \
  --buckets /srv/sharingan/models/buckets-200 --players 6 --iters 0 \
  --threads 8 --mem-gb 0.001 --learner-sizing adversarial --eval-deals 1 \
  --seed 5 --lbr-deals 20000 --out hardening-05/lbr-20k.json
```

**Results:** strong **+841.4 ± 161.7 mbb/game**; weak **+330.0 ± 164.8**. Both find
significant profit against fixed C@2e10. This is a new attack bound, not an A/B improvement
of the defender. The one-deal ABR placeholder in the JSON is irrelevant and its zero CI
must not be interpreted. Runtime 1,938.9 seconds; log `hardening-05/lbr-20k.log`, immutable
driver `hardening-02/lbr-20k.sh`. Standard LBR already exists in the repository; no new
policy code to keep or revert. Independent-seed and SHARINGAN-2 confirmation remain open.

### Hardening iteration 6 — passive mass versus actual call price (2026-11-04, reverted)

**Outcome: revert.** The correction defeats the old frozen attacks and improves strong
LBR, but a freshly trained attacker scores significantly better against it than against
the baseline on both independent deal seeds. That attacker still loses; nevertheless,
this fails the predeclared no-significant-regression requirement. No playing-policy
change from this experiment is retained or deployed. The detailed positive and negative
measurements below are retained together.

**Hypothesis.** The frozen attack's profit without hand-offs may exploit the difference
between actual and shadow call prices after off-menu translation. Test a parameter-free
correction with `q = call / (pot + call)`. When actual q is cheaper, multiply abstract fold
mass by `q_actual/q_shadow` and transfer the removed mass to call. When actual q is more
expensive, multiply call mass by `q_shadow/q_actual` and transfer the removed mass to fold.
Leave raise mass and equal-price decisions unchanged. This is a heuristic, particularly
in multiway pots; the experiment must establish its value rather than assuming it.

**Candidate:** an opt-in `TrackOpts::price_aware` (default false), applied consistently in
bench, search, and the exploiter's training target. CLI selector `--price-aware`, offline
arena selector `ARENA_PRICE_AWARE=1`; neither live configuration nor defaults are changed.
Three new tests cover mass/price invariants, training/evaluation parity on translated
histories, and search/bench parity. Required five-package release tests passed on the
isolated Platinum worker: **148 tests / 27 suites**, zero failed or ignored; all five
requested builds passed (`price-validation.log`, `price-validation.ok`). Same nice/affinity/
isolated-environment constraints as iteration 4. Native runtime binaries are built on the
original host, nice 10, eight build workers, alongside the unchanged 11-worker 3M learner.

**Predeclared A/B:** use the *same saved 1M attacker* and identical deal seeds in both
arms. Before examining candidate results, require the disabled new binary to exactly
reproduce all frozen reference estimates and learner miss rate at seed 271829, 100,000
deals/seat. Failure stops the queue for investigation. Then evaluate both arms at 200,000
deals/seat with seed 271829 (discovery). Proceed to the reserved seed 314160, again
200,000 deals/seat per arm, only if the candidate's upper 95% bound is below the baseline's
lower bound. Apply the same disjoint-CI criterion to confirmation. Do not increase sample
counts or tune the formula after observing this candidate. A failed gate means revert or
report an inconclusive result; it does not authorize retaining an unproved defending change.

Native queue `~/fix/hardening-06/price-ab.sh` (launcher PID 707978), logs/JSON under
`hardening-06/`. Build command from `~/fix/sharingan`:

```sh
env RAYON_NUM_THREADS=8 CARGO_BUILD_JOBS=8 /home/ubuntu/.cargo/bin/cargo build \
  --release -j 8 -p sharingan-arena -p bench -p exploit -p search -p sharingan-serve
```

Copy immutable executables to `hardening-06/exploit-price` and `arena-price`; record their
SHA-256 in `binary-sha256.txt`. Evaluation command from `~/fix` (queue inherits nice 10):

```sh
./hardening-06/exploit-price --compact \
  --target /srv/sharingan/models/sharingan-1.plcp --sizing medium \
  --buckets /srv/sharingan/models/buckets-200 --players 6 --threads 11 \
  --learner-sizing adversarial --load-learner hardening-02/learner-1m-confirm.bin \
  --seed 1 --eval-deals 200000 --eval-seed 271829 \
  --out hardening-06/discovery-base.json
```

Candidate adds `--price-aware` and writes `discovery-price.json`; holdout substitutes
314160 and writes `holdout-base.json` / `holdout-price.json`. Replay uses 100000 deals and
writes `replay-disabled.json`. All evaluations are 11 workers; with the 3M run, total
compute workers remain at most 22, and only C@2e10 is used. This queue stops after the
frozen-attacker gates. Keeping a playing-policy change still requires probe/control A/B,
adaptive attack/LBR checks, final-model confirmation and the normal-strength head-to-head
gate. No defending-policy improvement is claimed yet.

**Discovery result:** baseline **+447.4 ± 67.3**, price-aware **−2,849.2 ± 52.4**
mbb/game for the frozen attacker, 200,000 deals/seat per arm. Raw winnings are +449.2 ±
80.2 and −2,848.7 ± 73.2 respectively. The disabled-binary 100k replay exactly matches
the saved reference, all seats/raw estimates and learner miss rate included. Native builds
passed. Binary SHA-256: exploit `025d7aabb5cf7f5fac2cbd4a3928cf2f1ea27a197996225742c09a5d6c5b36d2`,
arena `d8070111757b534f4e0d544a66bc06bc79404ec6bcff74aca8da0b28d9476f23`.
**Confirmation:** seed 314160, 200,000 deals/seat: baseline **+459.4 ± 67.5**, candidate
**−2,863.1 ± 52.6**. Both predeclared frozen-attacker gates pass. This demonstrates that
this frozen attack loses under the candidate, not that an adaptive attacker cannot exploit it.
Confirmation raw winnings: baseline +444.7 ± 80.6, candidate −2,882.6 ± 73.4. All six
candidate seat estimates have negative upper 95% bounds (seat selection is unnecessary).

**Broader gates queued before confirmation results:** `hardening-06/price-broad.sh`
(launcher PID 713889) waits for both frozen gates and stops if either fails. Run fixed-track
baseline (`ARENA_PRICE_AWARE=0`) versus candidate (`=1`) with the immutable arena binary;
`ARENA_TRACK` unset in both. In order: offline quickstart, tag, sombra, and the six
`probe_*.py` fixtures. Both arms use `--hands 20004 --lanes 8 --seed ab-20004 --local
--track sharingan-1`, C@2e10/medium/tables buckets. Each JSON must have all 20,004 hands,
zero forfeits/crashes/timeouts/illegal actions, positive lane decisions, and matching deal
digests. Stop at any attacker whose candidate lower CI exceeds its baseline upper CI.
Never import the old live-client `ctl_quickstart.py`.

If external gates pass, run the iteration-5 20k LBR command against `exploit-price` with
`--price-aware`, same seed 5/eight workers, output `lbr-price-20k.json`. Stop on a
disjoint-CI regression in either configuration. If that passes, train a fresh adaptive
attacker: the iteration-1 command with `--iters 1000000 --threads 11 --mem-gb 15 --seed 7
--eval-seed 161803 --eval-deals 100000 --price-aware --save-learner
hardening-06/learner-price-1m.bin --out hardening-06/train-price-1m.json`. Save the weights
for later cross-policy and independent-deal confirmation; fresh training itself is not a
paired policy comparison. The queue is sequential, with at most 11 workers alongside the
existing 11-worker 3M job; it does not touch the final model or any live service.

**External A/B completed:** 20,004 hands per arm, duplicate + AIVAT, identical
`ab-20004` deals. Positive means the attacker wins. Every run passed the validity checks
above, including zero failures and matching deal digests. All baseline scores reproduce
the earlier fixed-policy measurements at the displayed precision.

| attacker/control | fixed baseline | price-aware candidate |
|---|---|---|
| offline quickstart | −596.4 ± 135.2 | −857.7 ± 148.1 |
| tag | −465.6 ± 118.5 | −409.9 ± 117.7 |
| sombra | −419.6 ± 161.6 | −332.3 ± 156.5 |
| translation midpoints | −4,173.8 ± 573.4 | −4,743.5 ± 543.0 |
| min-bet | −5,983.4 ± 637.3 | −8,715.7 ± 639.9 |
| min-raise war | −8,937.9 ± 735.9 | −11,775.4 ± 782.6 |
| overbet (3x pot) | −661.6 ± 197.2 | −675.8 ± 186.2 |
| random sizes | −5,745.0 ± 485.7 | −6,528.6 ± 444.6 |
| tiny bets / river shove | −1,801.8 ± 353.0 | −1,933.4 ± 331.9 |

No disjoint-CI regression; min-bet and min-raise war improve with disjoint CIs. Other
changes are inconclusive under the predeclared criterion. These controls can use off-menu
sizes; they do not by themselves establish strictly on-menu invariance. Files use each
bot's basename and `-base.json` / `-price.json` under `hardening-06/`; sombra's basename
is `bot`. LBR is now running; adaptive retraining and final-model gates remain pending.

**Strict on-menu gate (added before its measurements).** Add
`arena/bench/tests/price_on_menu.rs`: feed both configurations identical medium-menu
histories and compare action lists and every exposed probability bit for bit, including
queries with another valid private hand. A filled-source fixture covers heads-up/six-max
replays and requires identical same-seed AIVAT/raw/per-seat control match estimates.
Its source always has a distribution, so it cannot introduce off-menu fallback actions.
The separate real-model test replays 20,000 six-max strict-menu hands, then evaluates a
uniform medium-menu control against five model copies on 100,000 deals/seat, both arms,
seed `0xA11CE`. Compare the resulting CIs using the same no-disjoint-regression gate.
The actual model's fallback is unchanged and can itself choose off-menu sizes; therefore
the full real-model match is measured rather than assumed identical. This control does
not replace the final-model normal-strength head-to-head gate.

The model test is explicitly ignored in the model-free suite and must run separately on
the original worker. Native driver `hardening-06/strict-native.sh` (PID 728815), nice 10,
CPU affinity 20–22, two build jobs and three Rayon workers. With existing 11-worker
training and 8-worker LBR, at most 22 compute workers. Commands from `~/fix/sharingan`:

```sh
RAYON_NUM_THREADS=3 CARGO_BUILD_JOBS=2 /home/ubuntu/.cargo/bin/cargo test \
  --release -j 2 -p bench --test price_on_menu -- --nocapture --test-threads=1
PRICE_TEST_MODEL=/srv/sharingan/models/sharingan-1.plcp \
  PRICE_TEST_BUCKETS=/srv/sharingan/models/buckets-200 \
  PRICE_TEST_OUT="$HOME/fix/hardening-06/strict-control-c.json" \
  RAYON_NUM_THREADS=3 CARGO_BUILD_JOBS=2 /home/ubuntu/.cargo/bin/cargo test \
  --release -j 2 -p bench --test price_on_menu -- --ignored --nocapture --test-threads=1
```

Logs `strict-unit.log`, `strict-model.log`, `strict-queue.log`. The isolated Platinum
worker also reruns the full five-package release suite and five builds with this fixture
(`run-strict.sh`, PID 2495922, `strict-validation.log` / `strict-validation.ok`), same
four-CPU affinity, nice 10 and clean environment as before. No defending runtime code
or experiment formula changed after observing the earlier A/B.

**Strict gate results:** 362,440 exposed probability queries on 20,000 real C@2e10
strict-menu histories match bit for bit, with zero after-off-menu classifications or
desyncs. The real-model control A/B (100,000 deals/seat, 600,000 hands per arm) is also
exactly identical in both arms: AIVAT **−3,554.0 ± 105.5**, raw **−3,634.1 ± 110.0**
mbb/game for the control. Output `strict-control-c.json`; native explicit model test
passed in 163.0 seconds. The full updated five-package suite passed **149 tests across
28 suites**, one deliberately ignored model-dependent test (run separately and passed
as above), zero failures. All five release builds passed (`strict-validation.ok`).

**Extra-capacity attempt:** existing Platinum budget has a $3/hour configured compute
cap and a current $1.21/hour primary. An isolated 8-CPU/128-GB model-validation worker
would cost about $1.64/hour, combined $2.85/hour before disk, permitting final-model work
without evicting C@2e10 on the original host. The initial `ptxp scale 1 --yes --no-input
-o json` setup failed because its versioned Linux CLI release artifact returned HTTP 404;
the platform removed the failed `main2-1` machine. Read-only inventory confirms only the
original primary remains. The primary's installed Linux CLI version differs from the
Mac version; its public release artifact also returns 404. Investigate whether a fork
from that live root can reuse its already-installed CLI, without updating or restarting
the primary. No model-validation worker or additional hourly capacity is claimed yet.

**Standard LBR A/B completed:** same seed 5, 20,000 deals/seat, 120,000 hands per
configuration and arm. Strong: baseline **+841.4 ± 161.7**, candidate **+462.9 ± 179.3**
mbb/game, a disjoint-CI improvement. Weak: baseline +330.0 ± 164.8, candidate +232.6 ±
168.3, inconclusive difference. Both candidate configurations still find significant
profit against fixed C@2e10; the candidate does not close every tested attack. Runtime
1,874.8 seconds; `lbr-price-20k.json` / `.log`. The same-seed one-deal ABR placeholder
again must not be interpreted. The queue passed its LBR gate and started adaptive
1M/15-GB training with the predeclared seed/menu/budgets; the policy is still uncommitted.

**Worker recovery in progress:** invoking the existing Linux CLI from `main2` with
`ptxp scale 1 --from main2 --yes --no-input -o json` successfully forked `main2-1`
without downloading a missing release. Inventory/context verified zero sessions/agents
there and a separate `/work` disk; the original primary remained unchanged. Disk growth
to 500 GB succeeded. Requesting the larger CPU/RAM profile from inside the new worker
stopped that worker during its restart path and returned `sandbox not running`; inventory
still shows the old 16-CPU/64-GB profile, stopped. Recover via the existing Linux CLI on
the primary, targeting only `main2-1`. Do not claim the planned 8-CPU/128-GB capacity or
launch model jobs until the running state and actual resources are verified.

**Verified fallback capacity:** the external resize also stopped the new worker without
applying the larger profile. Recovering it with the existing Linux CLI's `ptxp up --box
main2-1 --no-input -o json` succeeds at **16 CPU / 64 GB / 500 GB disk**. Use that verified
profile; do not keep retrying the larger resize. The primary remains `main2`, unchanged
at 16 CPU/64 GB with its existing sessions/agents. Combined running compute is about
**$2.43/hour**, within the configured $3/hour cap. This dedicated worker is for trusted
Rust/model validation; do not run the third-party sombra code as its `dev` user.

Start pinned, unauthenticated public-weight downloads on that worker under
`/work/sharingan-model-validation`, never copying credentials. Exact launch:

```sh
setsid nohup nice -n 10 taskset -c 0-1 env -i \
  PATH=/usr/local/bin:/usr/bin:/bin TMPDIR=/work/sharingan-model-validation/tmp \
  /usr/bin/python3 -I -u /work/sharingan-model-validation/download.py \
  > /work/sharingan-model-validation/download.log 2>&1 < /dev/null &
```

Actual Python PID **1059** (launcher shell 1052), confirmed live/nice 10. The task-owned
script and `public-manifest.json` live alongside the log. SHARINGAN-2 public revision
`c97c4736260db5b34d5e6ebd0a1544fccb401cf1`; C@2e10 public revision
`7cc90baa6c4350692668dd70e29601d5590a0d71`. Download SHARINGAN-2 then C sequentially;
no inference jobs run during this preparation. Each 25-GB part is resumed with curl,
checked against its published LFS SHA-256, appended to the assembled file, checkpointed,
then only that task-created scratch part is removed. Verify the final assembled SHA-256
against each pinned `SHA256SUMS.models` before renaming it into place. Checkpoint/lock
files prevent a second writer or mixing revisions. No giant local-Mac model copies.

The bucket archive's SHA-256 **72c46597e63c6a245037d65989e72e8d03e76078aa758f149a9b2c61694227bc**
passed before safe extraction under the task's `data/` directory. The first model part is
downloading; model identity, cross-host frozen replay and final-model results remain
unproven until their verification completes. Full models and large outputs stay on the
worker's `/work`, not the shared home. Inspection found the fork did not include the
primary's mounted source/build cache; materialize a curated source snapshot and build
runtime binaries on the new worker. The original Mac remains edit-only.

**Normal-strength harness preparation:** add per-side `--price-aware-a` /
`--no-price-aware-a` and corresponding `-b` selectors to `pluribus h2h`, with explicit
`track_a` / `track_b` fields in its JSON. Three tests cover independent sides, explicit
baseline selection, global/per-side precedence and conflicting flags. Preserve the
existing default when selectors are absent. Keep C@2e10 as the fixed reference in both
arms; changing both models' wrappers would obscure the intended comparison.

The first build launch failed before compilation because the expected source directory
was absent. Do not treat its launcher as a successful build. Copy a **192-file / 517,030-byte**
source-only archive (SHA-256
`5c8df09e200e21cf6ec80d223718dcbeb4056d0716e622a1382a79b0a8963c25`) through
the primary's existing local-file connection into a uniquely named small `~/.pt/xfer/`
artifact. Verify that hash before safe extraction into the new worker's task-owned
`/work/sharingan-hardening-1c51-audit`. No `.git`, environment/credential files, models,
website or `docs/research/`. Large models remain outside the shared home.

Corrected driver `run-h2h-tests-v2.sh` (PID **2726**), log
`/work/sharingan-model-validation/h2h-validation-v2.log`: nice 10, affinity CPUs 2–9,
clean environment and isolated Cargo/TMP/target directories. Run the required five-package
release suite **plus pluribus**, eight build/Rayon workers, then build the six packages
and copy immutable `pluribus-price`, `exploit-price`, `serve-price` binaries into the
model-validation directory, recording their SHA-256. Model download retains CPUs 0–1.
This changes only the measurement selector/metadata; the defending formula is unchanged.

**Predeclared normal-strength measurement:** after full model checksums, runtime tests,
cross-host frozen replay and a 200-deal mechanical smoke test, run both head-to-head arms
on **4,000,000 identical deals per format**, seed 1, 6-max/medium/tables, full duplicate +
AIVAT, all three existing formats. Use one head-to-head process at a time on the dedicated
worker, with no competing model job. Baseline command (paths relative to its task directory):

```sh
nice -n 10 env -i PATH=/usr/local/bin:/usr/bin:/bin ./pluribus-price h2h \
  --a models/sharingan-2.plcp --sizing-a medium \
  --b models/sharingan-1.plcp --sizing-b medium --buckets data/buckets-200 \
  --players 6 --deals 4000000 --threads 12 --seed 1 \
  --no-price-aware-a --no-price-aware-b --out h2h-base-4m.json
```

Candidate substitutes `--price-aware-a` for `--no-price-aware-a`, keeps
`--no-price-aware-b`, and writes `h2h-price-4m.json`. Do not use `--warm` on the smaller
worker. Verify explicit track fields; investigate a baseline inconsistent with the
historical +5.09 ± 1.16 mixed-format estimate before trusting a new result. A significant
regression is a candidate upper CI below the baseline lower CI for the first two formats,
or a candidate lower CI above baseline upper CI for the mirror (whose first side is C).
This test is still pending; strict-menu control invariance does not replace it.

**Expanded runtime validation completed:** required five-package suite plus `pluribus`:
**155 tests / 32 suites**, zero failures, one deliberately ignored large-model test
(separately passed earlier on the original worker). All six package builds passed.
Immutable worker binaries: `pluribus-price` SHA-256
`dc42183573a576fa5bcfbd1e529e98191f8df81a64edcb30bc557c0213543d93`,
`exploit-price` `bb73ac665be30dbf2870a2a6848e176df6115be3b66cdcf810fe9b3dfe6e46ef`,
`serve-price` `0abf1075be73435ac5d25cd30ab1746101a5845cc5f18eb95684d4aa2e0216de`.

**Stronger frozen-attack gate:** after the original 3M training/holdout driver finished,
`hardening-06/3m-price.sh` (PID 751960) first required exact disabled-binary replay of
the saved 3M attack, all per-seat/raw/pooled estimates and miss rates included. It passed.
Then the same archive and same 100k-deal seeds gave:

| evaluation seed | fixed baseline | price-aware candidate |
|---|---|---|
| 271828 | +2,325.2 ± 106.5 | −2,589.2 ± 77.5 |
| 314159 | +2,288.3 ± 106.2 | −2,512.3 ± 76.8 |

Both improvements have disjoint CIs. Pooling only these evaluations of the same 3M
archive gives baseline **+2,306.7 ± 75.2**, candidate **−2,550.7 ± 54.6**; candidate raw
−2,476.8 ± 77.8. All units mbb/game for the attacker. Exact candidate commands are the
iteration-2 reload command with `--price-aware`, seeds 271828 and 314159, outputs
`hardening-06/3m-price.json` / `3m-holdout-price.json`. Comparison and pooled artifacts:
`3m-price-comparison.json`, `pooled-price-3m.json`; both frozen 3M gates passed.

**Adaptive training completed:** 1M iterations / 15 GB, seed 7, all six seats, accurate
configured price-aware target fallback. Reported 4,103.3 seconds; 7,730,927,301 traversal
nodes, 268,331,773 infosets, 1,875,000,000 slots; store full, miss rate 11.575% in the
initial evaluation. The trained attacker loses **−1,094.8 ± 50.9**, raw −1,132.2 ± 66.5,
on its predeclared seed 161803 / 100k deals per seat. This negative result alone does not
establish robustness or a best response; the finite store still has the documented
learner fallback mismatch. Its native weights are saved as `learner-price-1m.bin`.

**Adaptive A/B protocol:** `adaptive-1m-ab.sh` (PID 784153), nice 10 / 11 workers, first
replays that saved policy against the candidate on seed 161803 and requires exact
pooled/per-seat/raw estimates, confidence intervals, counts and miss rate. That check
passed. Evaluate this same frozen policy against baseline on seed 161803, then both
arms on the independently reserved seed 271830, 100k deals/seat per arm throughout.
Commands use `--load-learner hardening-06/learner-price-1m.bin --seed 7 --eval-deals
100000 --threads 11` and `--no-price-aware` / `--price-aware`; other C@2e10/medium/
adversarial/tables arguments are unchanged. Any candidate lower CI above baseline upper
CI is a regression, even if the attacker loses in both arms. No sample-size tuning.

| evaluation seed | baseline, adaptive attack | candidate, same adaptive attack |
|---|---|---|
| 161803 | −1,321.0 ± 51.9 | −1,094.8 ± 50.9 |
| 271830 | −1,350.3 ± 52.4 | −1,133.2 ± 51.3 |

**Both adaptive seeds fail the regression gate.** The prototype yields about 226 and
217 mbb/game more to this attacker; neither result claims profitable exploitation.
`adaptive-1m-comparison.json` records both failures and the driver stopped. Do not weaken
that gate after seeing the much larger improvements against the old frozen attacks.
Cancel the candidate's planned final-model/4M head-to-head promotion checks: a failed
gate already rules out retaining it. Retain the trained policies for later regression
experiments and final-model *baseline* attack confirmation.

**Source rollback:** restore all seven modified tracked Rust files to main and remove
only the task-created `arena/bench/tests/price_on_menu.rs` (hash checked before removal,
locally and on the original worker). `docs/research/` is untouched. Preserve the failed
prototype as `hardening-06/sharingan-h06-retired-source.patch` and `.tar.gz`; source-archive
SHA-256 **681807acb2aa92043b4ff6143b9e252a1c352dcaf2a99f31b05d615a692a1cb4**.
The immutable experiment executables and full 192-file source snapshot remain available
for reproduction. Main has no price-aware option/correction from this attempt.
Restored validation driver `hardening-06/reverted-tests.sh` (PID 795826), nice 10,
affinity CPUs 0–15, eight build/Rayon workers: rerun the required five-package release
suite plus pluribus and build all six packages. **148 tests / 31 suites passed**, zero
failed or ignored; all six builds passed (`reverted-validation.log`,
`reverted-validation.ok`). No restored-source policy improvement is claimed.

**Transport and identity checks:** the first private command relay truncated an 8-MiB
archive chunk (the direct command path is capped at 256 KiB of output). Its byte-length
guard rejected the payload before appending anything, and PID 13894 exited. Do not use
command output as a large-file transport. The worker had no active local-file connection;
start its per-worker link **from the Mac**, `ptxp link --box main2-1 -o json`, preserving
the existing policy revision 2. Read-only status confirmed both this connection and the
primary connection running; no permissions were broadened and no primary session was
stopped. Never run attach/link from a machine.

Prepare the original frozen 1M archive after the 3M gate (driver `prepare-transfer.sh`,
PID 755984): `zstd -q -T1 -c hardening-02/learner-1m-confirm.bin`, test the compression,
and record native/compressed/metadata SHA-256. Transfer its compressed file through a
task-owned Mac temporary directory using nice-10 rsync, then the standard chunked
`ptxp local cp` on the worker. Corrected `copy-attacker-v2.sh` (PID 16628) passed both
hashes and native-size checks:

* Native: 12,415,042,764 bytes,
  `169ef576454787e2add5fb519ff15606ba8d5bab2c752ba0a784ce6e9c485e35`.
* Compressed: 5,725,274,680 bytes,
  `ec0465646df34ce2db9b19dd0077657e3fff5381ef868d759fb1c0277bf14910`.

Only the copied archive manifest's bucket *path* is rehomed to the task's verified
bucket directory, with original metadata preserved and relocation hashes recorded.
Weights are byte-identical. Cross-host frozen replay is still required before using
that copy's match results. Remove only the task-created 5.7-GB Mac relay file after
remote verification; no heavy Mac build/training or full-model copies were performed.

**C model identity discrepancy (investigating):** nice-10/idle-I/O full SHA-256 reads
with `O_DIRECT` bypassed the original host's model page cache. SHARINGAN-2 exactly matches
the pinned public release: 169,906,172,488 bytes,
`0f57b29605cb0fc5f2ecf4c1fcb266df6321d8459784b1577a30d54bb94c5b8f`.
The provided `/srv/sharingan/models/sharingan-1.plcp` has 124,681,168,783 bytes but hashes
to **acf73d86e16f1b681f703c58bd880ae8cf39e84342750bcd1c82bef07560fc23**,
whereas its pinned public `SHA256SUMS.models` advertises
`2a4484f6fabf61470a67c941552f5a97c6f6474e14ea010ef94a41d7b5861300`.
Its first 128 header bytes match the public file. The part audit already confirms that
part 00 matches its public LFS hash, whereas part 01 differs (provided-file part SHA-256
`6d7a84f09234189f979869acf4efd624ba7ccf28cf3b76c507a4ed9447ac154f`).
Thus the provided C file is not byte-identical to the pinned public parts; the full
section audit remains in progress. All original C measurements above
used this same provided file and reproduced; no cross-host public-C number has been
trusted. Investigate before treating the public C copy as identical: direct-I/O
`sharingan-c-part-audit.py` (PID 808881) compares every provided-file part with the pinned
LFS SHA-256 values and records per-section hashes, without modifying any model or public
repository. The complete model/part audit and final-model attack confirmation remain
open; do not infer a cause for the mismatch from its size/header alone.

The extra worker rebooted during part 03 of the public C download; no numerical match
was running there. Autopilot remains disabled, and kernel logs are unavailable to the
worker user, so the cause is undetermined. The resumable downloader restarts only after
confirming its old process is gone; its single-writer lock and final SHA-256 gate remain
required. Rehash the verified SHARINGAN-2 file and transferred learner before use after
this interruption. No extra-worker match result has been accepted.

### Hardening iteration 7 — shadow parity and C input integrity (2026-11-04, investigating)

**Hypotheses.** The attack trainer shares one target shadow across all opponent seats;
evaluation keeps a shadow per seat and labels only that seat's own actions as own.
Check whether the difference changes queried target histories before changing the
trainer. Separately, investigate the C checksum discrepancy from iteration 6 rather
than trusting a numerically different input.

**Shadow diagnostic (no retained source change):** external task-owned Rust crates
`hardening-07/sharingan-shadow-parity-v1` and `-v2` compare the trainer's shared shadow
with six evaluation-style seat shadows, sampling concrete actions from the actor's
own shadow. Fixed translation options, 6-max / medium target. Compare target history
keys and sync reasons before every opponent decision. The model-free diagnostic does
not query a policy or estimate poker winnings.

* V1, seed 2026110407, uniform menu actions: 100,000 strict-menu hands / 753,323 target
  decisions and 100,000 adversarial-menu hands / 764,134 decisions; **zero differences**.
* V2, independent seed 2026110408, favor passive and smaller non-all-in bets to reach
  longer histories: 1,000,000 strict-menu hands / 17,464,126 target decisions and
  1,000,000 adversarial-menu hands / 19,010,808 decisions; **zero differences**.

Build and run with task-owned `sharingan-shadow-parity-run-v1.sh` / `-v2.sh`: nice 10,
`CARGO_TARGET_DIR=/home/ubuntu/fix/sharingan/target`, release/offline build, four workers
on CPUs 0–3; each diagnostic runs on CPU 0. Executables `shadow-parity-audit` /
`shadow-parity-audit-v2`, each with and without `--strict`. Source, build logs, output
logs and completion markers remain under `hardening-07/`. These samples do not prove
parity for all histories or randomized translation. They supply no reason to retain
a trainer change. Main playing code remains unchanged; its last required release
suite passed 148 tests / 31 suites, all six builds passed in iteration 6.

**Input audit, unresolved:** the original C part-01 direct hash differs from public.
Part 00 and part 02 match their pinned public LFS hashes. A complete 25-GiB part-01
comparison, using 1-GiB SHA-256 blocks and direct I/O on both machines, finds only one
differing GiB, starting at 28,991,029,248 bytes. Refine it to one 4-MiB block, then one
64-KiB block starting at **29,250,551,808 bytes** within the key index. At 4-KiB granularity,
five original-host pages hash as all-zero pages; the pinned public copy has ordinary
key bytes. Captured byte comparison shows exactly **20,480 zeroed bytes / 2,560 keys**
(20,440 byte values differ), with the remaining 45,056 bytes identical. The good/public
block is strictly sorted; the zeroed block has 2,559 adjacent sortedness violations.
However, another direct read of the *same original-file block* exactly matches
the public bytes. Thus a static different-export explanation is insufficient.

Same original-file 64-KiB range has returned SHA-256
`62b40adda8eadade3d02d51d05f6acde89f8eb768bf92f80f081b3b6f2fcf0d1`
(public/matching read) and
`50519d1c0058aa529e8f4a3f8c3b5330cb71686cf91ebd91e16971288e5fc664`
(five zero pages). Both Python `readv` with anonymous aligned buffers and native
`dd iflag=direct` read-only probes are used. A buffered read also returned a third
mixed-block digest. File inode 24912546, size 124,681,168,783, mtime and ctime remained
unchanged across the repeated-read probe. No claim about a physical cause is established.
Read-only `/proc/mdstat` identifies a two-member RAID1, both marked up; sysfs reports
clean, non-degraded, idle. **Do not initiate a RAID scrub/repair, alter /srv, or restart
services.** Hardware/configuration repair is outside this hardening experiment.

Exact probe commands and captured per-read JSON are retained in
`hardening-07/sharingan-c-read-stability-v1.py`, `c-read-stability-v1.json`,
`sharingan-c-part01-gib-audit.py`, `c-part01-gib-source.json`; matching public block
checks are on the isolated worker under `/work/sharingan-model-validation/`.
The first probe's good original-host block and public block are byte-identical;
keep small distinct read samples as evidence, never patch the original model.
The complete original-host part/section audit finished read-only: all four other LFS
parts match, and the full digest remains `acf73d86...`; section hashes are preserved
in `hardening-06/c-part-audit.json`. No new matches use that original C file.

**Independent inputs:** on the extra worker, all public C parts and its full digest now
match `2a4484f6...`. SHARINGAN-2 was fully rehashed after the reboot and again matches
`0f57b296...`; both copied native/compressed learner archives pass post-reboot rehashing.
Use the restored-main executable, SHA-256
`f3b5eaa4fbec93237bdf2ad75996ff42032e46d0d39b461cb9acee83715c4e77`.
Task-owned `public-replay-v1.py` (PID 68612) first requires those verification markers,
then evaluates the frozen 1M learner against public C, 100k duplicate deals per seat,
seed 271829, all six seats, medium target / adversarial learner, eight workers. Require
**exact** pooled/per-seat/raw estimates, CIs, counts, miss rate and track options versus
the original `reference-1m-original.json`. Any mismatch stops the driver for investigation.
**The public-C gate passed exactly:** +419.45226595696886 ± 95.36833763196319 mbb/game,
raw +466.9714500000008 ± 114.10739423294837; all pooled/per-seat estimates, confidence
intervals, counts, miss rate and track options match the original reference. This
confirms that frozen 1M attack on a fully verified independent public C model, despite
the original-host read anomaly. It does not retroactively validate every old training
traversal or the stronger 3M attack. `public-c-replay-gate.json` has all checks true.
Baseline SHARINGAN-2 evaluations of the same frozen policy have now started, seeds
271829 and 314160, again 100k deals per seat each. No playing-policy A/B or
promotion is implied. Commands, clean environment, target SHA-256 and boot ID are recorded
per evaluation; no simultaneous inference on two models. The already nice-10 driver
launches its child through `nice -n 10`, giving the child effective nice 19; eight CPUs
4–11 are reserved for that evaluation, within the worker's 16-CPU capacity.

**Private original-host input reconstruction (not a deployment):** task-owned
`sharingan-prepare-inputs-v1.sh` first runs `sharingan-reconstruct-c-v1.py`, nice 10 /
idle I/O. Create a fresh private incoming file under `~/fix/models/`, copy the original
file read-only, and replace only the known 20,480-byte key region in the *new copy*
with the byte-identical good/public block captured above. Require full-file buffered
**and** direct-I/O SHA-256 to equal the public release before atomically naming it
`~/fix/models/sharingan-1-verified.plcp`. If either check fails, retain evidence and stop;
never patch the original /srv file. Record old/new-region digests, source metadata,
full-copy digest and both verification results. This is reconstruction of a verified
experimental input, not a new poker strategy or permission to repair live storage.
Behind that success, prepare the frozen 3M archive for independent confirmation using
`sharingan-prepare-3m-transfer-v1.py`: single-thread zstd, source SHA-256 during compression,
an independent full direct-I/O native rehash, zstd integrity test and compressed SHA-256.
The native archive is 29,581,123,222 bytes; never transfer if its two source hashes differ.
No defending-policy fix is kept in this iteration so far.

**Documentation correction:** `arena/bench/TRANSLATION.md` still called Check the default,
recommended it using a pre-keep-sync B ablation, described check/call as the current
hand-off fallback, and said search used legacy histories. Correct those statements
against current `TrackOpts`, `Shadow`, bench/search fallback code, and the measured
experiments. Preserve the B tables explicitly as historical results. `docs/TRAINING.md`
now records that the 1.83e11 run finished on 2026-11-02. These edits change no executable
code or model; the prior 148-test / six-build restored-main validation still applies.
No new poker A/B claim is attached to documentation edits.

The full-budget run (C@final, 1.83e11 iterations, finished 2026-11-02) is SHARINGAN-2. 6-max, duplicate + AIVAT, 4M deals per format:

| test | 3 + 3 | 1 vs 5 | 5 vs 1 (other side) |
|---|---|---|---|
| SHARINGAN-2 vs SHARINGAN-1 (C@2e10) | **+5.09 ± 1.16** | +8.60 ± 1.89 | −7.64 ± 1.88 |
| SHARINGAN-2 vs C@4.77e10 | **+1.97 ± 0.97** | +3.76 ± 1.57 | −1.95 ± 1.58 |

Trained best response (12M iterations) vs SHARINGAN-2: the exploiter loses −247 ± 58 (−323 ± 62 against SHARINGAN-1). A true best response cannot lose to the strategy it exploits, so this exploiter is far too weak to measure exploitability: it finds no leak in either, and cannot tell them apart. Head-to-head play is the evidence that SHARINGAN-2 is stronger.


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
