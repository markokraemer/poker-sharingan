# SHARINGAN results

Everything measured so far, in one place. Conventions: 6-max no-limit hold'em, 100 bb, duplicate deals + AIVAT variance reduction,
mbb/game = thousandths of a big blind per hand (1 mbb/game = 0.1 bb/100), 95% confidence intervals. Blueprints: **A** = paper-style menu,
**B** = `local` menu, **C** = `medium` menu (final). See [EVALUATION.md](EVALUATION.md) for how each number is produced.

SHARINGAN-1 = C@2e10 + turn/river search. The inference API (`/v1/act`) plays it with search; the leaderboard's sharingan-1 track, `sharingan play --vs sharingan-1`
and the browser play page use the released C@2e10 blueprint without search. The search results below (+87 vs the blueprint, Open Poker live) are the search bot.
## Action-translation leak and fix (Nov 2026)

### Hardening continuation — finish gates and expand API attacks (2026-11-05–06)

The user resumed autonomous hardening; public access remains disabled. Resume the
immutable iteration-8 candidate without changing its formula, gates or old results.
Its mixed-passive correction reduced the final-model frozen 3M attack from
+2,543.1 ± 105.7 to +428.1 ± 100.0 at discovery and +2,382.6 ± 106.9 to
+251.6 ± 100.8 at holdout: substantial reductions, with remaining profitable play.
No policy promotion is justified until its unfinished checks pass.

`hardening-11/finish_passive_odds.py` runs as ubuntu/nice 10, at most 12 workers:
verify immutable executable hashes, buffered/direct full final-model hashes and
stable metadata, then exactly replay the existing frozen 1M reference. Complete
the interrupted enabled S2 LBR arm with the existing 20,000-deal seed-5 baseline
and identical command/configuration. Reuse of that completed baseline is explicit;
the exact frozen reference verifies model/executable behavior before comparison.
Run the already registered native one-chip-left attacker against S2, 3,334 duplicate
deals / 20,004 hands per arm, seeds 2026110410 and 2026110411, disabled/enabled.
Reject any significant raw or corrected defender regression under the original
disjoint-95%-interval rule.

Then verify private C in full and perform the original 200-deal mechanical h2h
smoke followed by both 4,000,000-deal, all-three-format arms at seed 1: S2 disabled
versus C disabled, then S2 enabled versus C disabled. Only this single prescribed
h2h process uses both models together; other model jobs are serialized. Require
the baseline mixed-format CI to overlap the historical +5.09 ± 1.16, investigate
any discrepancy, and retain all original raw/corrected normal-strength gates.
Fresh outputs go under `~/fix/hardening-11`; no old incomplete file is overwritten.
Failures stop the driver and preserve a traceback/commands/logs. All prior external,
adaptive and frozen-attack gates remain mandatory. No live services or models are
rebuilt or changed. API repair preparation and compilation may proceed separately,
but inference against C waits until the S2/h2h queue releases the model cache.

**API repair registration (before new measurements).** Keep iteration 10's
real-state fallback change isolated from mixed-passive odds: only the API's absent
distribution branch calls the existing fallback with real state/cards, without
scaling that action twice; its requested probability response is the actual
deterministic action with mass one. Extend the model-free royal-flush regression
to five blind scales, and add always-run authenticated HTTP and batch coverage.
The first source-only build omitted the compile-time SQL migration; it failed
before tests or poker evaluation. Preserve that failure and use the complete v2
archive, SHA256 a498ec3c2cc65918c507cd301e70c5999d57808ce5fcd52aa31c98af2b9f5190.
Required seven-package release tests/builds run remotely with four workers.

The extended API measurement harness must exactly replay the original near-all-in
harness's baseline score/per-seat statistics/diagnostics before interpreting its
results. Primary attack remains BB 1000, one chip left, seeds 2026110410/2026110411,
3,334 duplicate deals / 20,004 hands per arm; require a profitable baseline lower
bound above +1,000 and disjoint improvement intervals on each seed. Test boundaries
2/5/6/10 chips at both seeds, and one chip at BB 100/200/2000/10000. Preserve the
10-chip exact-grid and BB-100 native controls exactly. All comparisons reject a
significant raw or corrected defender regression; no post-result sample tuning.

Run all original nine SDK bots through actual Core::decide (search disabled),
plus nit/TAG/LAG/maniac/station/random, at BB 100 and 1000, seed 2026110411,
20,004 hands/arm. The SDK adapter independently reconstructs chip/history totals
and asserts them against each native public state. Keep a hand in one bot process,
preserve its original code/memory and SDK legalization, abort on timeout/bridge
failure, and reject logs containing caught bot tracebacks. Sombra remains ubuntu
only, no-new-privs, empty HOME and credential-free environment. Canonical outcomes
must match exactly for deterministic bots; Sombra's adaptation and the seeded
random probe have scheduling-dependent process histories, so use their declared
raw/corrected regression gates rather than claiming exact replay.

Final S2 API runs share only S2 cache with the native LBR stage (four API workers
plus at most four SDK processes; <=20 total computing workers including builds).
They stop if the native queue advances to C or an input/reference gate fails.
S2 results remain provisional until the new baseline also exactly reproduces the
historical C API positive and exact-grid controls, and C passes the same repair
checks. No API result, source-only build or synthetic nuts regression alone
establishes a proven poker improvement. No live API is restarted or exposed.

**API release validation:** the seven-package release suite completed with 180
reported passes across 38 suites, zero failures/ignored tests. As in the earlier
API audit, 17 existing fixture-dependent tests return early because no tiny model
fixture is installed; 163 tests actually executed. All five mandatory model-free
regressions executed, including the new authenticated single/batch HTTP test
(25 single requests plus the same 25 requests in a batch). Separate always-run
4,000-request legality/probability fuzzing is added to cover mixed player counts,
deep/short stacks and exact/inexact blind scales; its result is still pending.
No new poker measurement is claimed by this release-suite result.

**Additional probability-reporting defect (before its repair):** the new
model-free real-chip HTTP fuzz test failed on generated request h-78010 at BB 30:
the actual raise was 2,520 after the stack guard capped a proposed 3,060, but
`probs` still assigned probability one to 3,060. The original 4,000-case run stops
at that first failure; it does not establish a failure rate over all cases.
The API poker harness explicitly marks target decisions unexposed, so it never
uses these metadata for AIVAT. The separate probability repair must apply the
same cap/legality mapping to every reported distribution entry and aggregate
actions that map to the same real action, without changing action selection.

Preregister a 512-request model-free deep-stack matrix (six blind scales and
button positions, 200–711 BB stacks) using a pure-shove source: baseline must
show all 512 action/probability mismatches and pass the uncapped control. After
the smallest reporting-only change, require zero mismatches, all sampled actions
and all other non-timing/non-probability response fields byte-identical, and the
uncapped control unchanged. Report Wilson 95% intervals for this synthetic error
rate, never as poker EV. Preserve both source snapshots, commands and response
arrays. Then rerun the 4,000-case HTTP matrix and required release checks.
No reporting fix may be counted as a poker-strength improvement.

**Reporting A/B target passed, release gate pending:** after correcting a fixture
assumption about zero-probability entries (which remain present in the API), the
baseline reproduces 512/512 mismatches and its uncapped control passes. The
reporting-only candidate gives 0/512, with every non-timing/non-probability response
field identical. Wilson reference intervals are [99.255%,100%] and [0%,0.745%]
over the fixed synthetic sample; these do not estimate production frequency.
The two-raise coalescing and deterministic-fallback probability tests also pass.

With that correction, all 4,000 generated HTTP requests pass legality/probability
checks, but the initial test then fails its own coverage assertion: none happened
to enter the outer API fallback. Do not call that first run a passed test. Keep
its log and add the known scaled-terminal request explicitly, without weakening
the legality/probability checks. Final v2 source archive
68a58971c06fbf52535776a526184692059e06b0388e4f62aafb19b47c1d8419 changes
only that additional test relative to v1. Rerun targeted checks, the 4,000 generated
plus one explicit HTTP case, and the required seven-package release suite/builds.
The production reporting code is unchanged between those candidate versions.
No reporting change is committed until the complete gate passes.

**Reporting iteration outcome — kept:** v2 passes the 512-case A/B again, the
uncapped/coalescing/fallback controls, and all 4,000 generated plus one explicit
authenticated HTTP cases. Generated coverage: preflop/flop/turn/river
2,445/870/440/245; 2/3/4/5/6 players 726/779/832/849/814. The required seven-package
release suite reports 184 passes / 39 suites, zero failures/ignored; excluding
the 17 existing fixture-dependent early returns, 167 tests executed. All release
builds pass. Commands, source hashes, both response arrays and A/B/validation
receipts are preserved under ~/fix/hardening-11/probability-{baseline-v2,fixed-v2}.
Main's changed Rust/test files match the validated source hashes exactly.

The production change only reports guard-mapped probabilities and the existing
deterministic fallback action; sampling/action selection is unchanged. It does
not repair the API poker leak or change native blueprint/search strength, so a
new poker EV or native h2h result is not attributed to this reporting fix. The
separate real-state action repair and mixed-passive policy remain uncommitted
pending their own complete gates; public access remains disabled.

**Separate API action-repair preliminary S2 results:** the extended harness exactly
replays both original-harness control outputs before its first new comparison.
At BB 1000 / one chip left, 20,004 hands per arm:

| evaluation seed | baseline attacker | real-state fallback candidate |
|---|---:|---:|
| 2026110410 | +13,900.5 ± 888.7 | −2,325.7 ± 452.0 |
| 2026110411 | +14,532.0 ± 918.7 | −2,757.5 ± 472.1 |

Both raw and corrected primary improvements have disjoint intervals. These API
target decisions are unexposed; never pool with native full-decision AIVAT scores.
The two-/five-chip variants show the same repair effect, but six-/ten-chip
exact-grid variants remain profitable (about +4,145 ± 814 on holdout), unchanged
by this repair. Full SDK/built-in controls, historical C replay and the C matrix
remain pending; no API poker improvement is retained yet. The Sombra API sample
also needs follow-up: its canonical BB-100 baseline is +198.9 ± 111.3 and
candidate +240.4 ± 110.2, unlike the different-seed native worker loss. These
are distinct sampling/adapter paths, not a paired contradiction or proof of a
new bug; require native/API distribution parity and independent confirmation
before interpreting this as an additional Sombra attack bound.

**Stronger attacker variations registered before training:** use the existing
task-owned PTXP worker main2-1 (16 CPU / 64 GB / 500 GB), which was already running
and idle with no terminals/agents. Do not change its size or the busy primary.
Only trusted Rust runs there; no third-party/Sombra code or credentials are copied.
New boot 4c96fcf0-f7a0-4dd4-9f7a-e7fab5c09202 invalidates the old input certificate:
first fully hash private C using buffered/direct reads, hash the frozen reference
learner, and exactly replay its 100k-deal/all-seat reference (+419.45 ± 95.37).

Train three new frozen attackers against the opt-in mixed-passive candidate on C:
3M iterations each, 15 GB, 12 nice-10 workers: adversarial-menu seat 2 / seed 12;
adversarial-menu seat 3 / seed 13; and paper-menu every seat / seed 14. The two
focused seats are exploratory choices from the old C seat audit, not independently
selected final-model findings. Per-seat training concentrates the finite store;
the third study tests a different menu across all six seats. Existing full-store
learner training/evaluation fallback mismatch remains a limitation: record miss
rates/store saturation and never interpret a negative finite learner as robustness.

Each study saves its native archive, verifies buffered/direct archive hashes,
and requires exact enabled reload. Evaluate the same frozen policy against
disabled/enabled target arms at 100,000 duplicate deals per trained seat, discovery
2026110513 and holdout 2026110514, using both average and argmax extraction (fixed
within each A/B). Keep all estimates/per-seat/raw results and reject a significant
defender regression under the original disjoint-CI rule. After all C studies,
hash S2 in full on this boot, replay its frozen reference exactly, then run all
the same frozen A/B pairs on S2. Only one target model is used at a time per host.
These additional attack gates must finish before mixed-passive default promotion.
Keep all new archives; stop for disk headroom instead of deleting old evidence.
Exact commands/environments/hashes/logs go under
/work/sharingan-model-validation/hardening-11/training-v1. No public deployment.

**Supplemental API/native parity and phase scheduling (registered before its
results):** on canonical BB 100, instrument each API target decision with a
stateful native CompactPolicy queried on the identical generated public state,
hero cards and observation stream. Require maximum action-probability difference
<=1.01e-6 (the API rounds displayed probabilities) and report any sampling
differences under the API's seed transform. The instrumentation must first
exactly reproduce the existing 20,004-hand near-all-in control statistics and
decision/fallback/error counts. Then measure Sombra on independent seed
2026110515 / 20,004 hands, retaining the original SDK code/adapter and abort-on-error
rules. This distinguishes policy/replay disagreement from differing samples and
adapter histories; it does not change a playing strategy.

Insert the full original-host C API matrix before the predeclared 4M h2h gate.
The continuation controller waits until all S2 LBR/native arms finish. It may
terminate only its owned original Python coordinator while it has no inference
child and is partway through its C checksum pass; preserve every completed
result and record the intentional scheduling handoff. The replacement acquires
that run lock, requires completed S2 API/parity gates, verifies C in full from
scratch and exactly replays its frozen reference. Run the historical API
positive/exact-grid controls and every registered C API variant/SDK/built-in pair,
then the unchanged 200-deal h2h smoke and both 4M/all-three-format h2h arms. No
strategy/gate/sample count is weakened. It never interrupts inference, stops a
live service, or overlaps a separate model job with h2h. Fresh controller outputs:
~/fix/hardening-11/c-api-h2h-v1; C API outputs: api-c-v1.

**Residual-size attack search, registered before native size results:** the
profitable six-/ten-chip API controls motivate a broader frozen sizing attacker.
While S2 remains the only target in cache, after the existing LBR/native pairs
finish, search residual stacks of 1/2/5/10/25/50/100/200/500/1000/2000/3300/5000
chips at canonical BB 100. Use immutable native harness SHA 3a9c570e… with exposed
native probabilities, 3,334 duplicate deals / 20,004 hands per configuration,
discovery seed 2026110516, disabled/enabled mixed-passive arms. First exactly
reproduce the just-completed registered native one-chip reference, including
diagnostics, and repeat it exactly; require the baseline lower bound above +1,000.

Select the single residual size with the largest enabled-candidate discovery
mean (ties choose the smaller residual). Freeze it and confirm both target arms
on untouched seed 2026110517 / 100,000 duplicate deals per seat / 600,000 hands.
Report every exploratory size and the independent confirmation separately.
Any disjoint raw/corrected defender regression rejects the mixed-passive candidate;
do not proceed with its h2h promotion work after such rejection. A 40-minute job
deadline preserves scheduling; incomplete output is not a passed attack gate.

Before it performs any handoff or poker measurement, retire the waiting v1 C
coordinator and preserve its log. V2 adds this serial S2 search before C hashing,
then runs the same full C API matrix and unchanged h2h gates. Fresh paths are
c-api-h2h-v2, native-size-sweep-v2, and api-c-v1. The sole size-search process uses
12 nice-10 workers, with no other model job running on that host. It changes no
policy formula, existing gate, public service or result sample retrospectively.

**Continuation results and expanded attacker registration (2026-11-05):** final S2
LBR is complete: weak +137.2 ± 170.0 to +135.4 ± 170.4; strong +795.0 ± 165.2 to
+704.4 ± 166.4 for baseline/mixed-passive. Neither comparison is conclusive.
The residual search selected 500 chips (5 BB). Its untouched 100k-deal/all-seat
confirmation gives +3,277.1 ± 145.8 baseline and +1,268.2 ± 138.2 candidate;
raw +3,296.6 ± 200.8 and +1,300.2 ± 196.9. The candidate significantly reduces
this attack but remains clearly profitable. In its blind seats the candidate
attacker earns +9,824.6 ± 289.5 and +10,070.0 ± 288.3; other seats lose. These
position estimates motivate a separate attack that folds outside the blinds,
not a claim that the pooled all-seat result already measures that strategy.

API/native parity passes on identical canonical histories: near-all-in 115,601
target decisions, maximum displayed-probability delta 4.9994e-7, zero sampled
action differences; independent Sombra 151,479 decisions, maximum 4.9962e-7,
zero sampled differences. The latter API-format sample earns +162.1 ± 109.0
(raw +181.5 ± 129.5), seed 2026110515 / 20,004 hands. It confirms moderate profit
under this adapter and sampling sequence. The original SDK worker negative
sample uses a different deal/button/participant-history sequence. Do not replace
one with the other or state that the fixed policy defeats every Sombra variant.
The full S2 API matrix passes all declared SDK/built-in regression gates; the
verified C matrix is running, after full buffered/direct hashes and exact frozen
reference reproduction. Public access remains disabled.

The extra 64-GiB worker passed its C input/reference checks, but its first
3M training attempt achieved only about 20–25 iterations/s due to insufficient
cache. Stop only that owned process group before completion and preserve its
logs; there is no usable new learner archive or completed attacker measurement.
Attempted larger profiles fail the service's 65,536-MiB RAM limit. The owned
worker is stopped; the busy primary is untouched. Relocate the three registered
studies to the original 188-GiB host rather than treating the incomplete queue
as passed. Use 11 workers there (at most 19 including four API workers and four
SDK processes), keep the original study seeds/menus/seat choices, and preserve
all frozen extraction, independent-deal and S2 confirmation gates.

Before those three studies, add a 3M-iteration / 35-GB attacker, seed 18,
focused on seats 0 and 1. Its attacker-only `residual-v1` menu contains every
adversarial-menu action plus legal raises leaving 1/2/10/100/500/1000/3300 chips
in the canonical 50/100 scale, with the same raise cap. Freeze the exact menu
definition in archive metadata and reject incompatible reloads. Add explicit
comma-separated seat selection, rejecting duplicates/out-of-range indices.
These changes alter attacker coverage, not target policies or model loading.
The first source build catches a u32/u8 comparison before tests; preserve the
failure and fix the casts. V3 source SHA256
79d877cc7039691b5d17acc5bf8dc800777ce0d3412917f182fecae057b3367f.

Required seven-package release tests/builds run for isolated default and
iteration-8-overlay variants. Before training, require both binaries to exactly
replay the old C frozen reference (normalize only the added disabled option in
tracking text). Train against enabled mixed-passive C, save native archives,
verify buffered/direct hashes and exact enabled reload. All four studies use
the registered 100k-deal discovery/holdout seeds 2026110513/2026110514, average
and argmax extraction, identical frozen policies across both target arms, and
both raw/corrected disjoint-interval no-regression gates. Keep focused-seat
statistics explicitly conditional; a full-table attack must also be evaluated
with its actual outside-seat folding policy. Preserve store saturation and miss
rates; finite learners with training/evaluation fallback mismatch cannot certify
robustness. Complete C inference before switching to S2, recheck its prior full
input certificate/metadata/boot and exact frozen reference, then confirm all
four frozen attacks. Source/commands/logs: ~/fix/hardening-12.

Freeze that full-table probe as 500 chips left in seats 0/1 and fold in seats
2–5. Its extended native harness must exactly reproduce the original all-seat
attacker statistics/diagnostics before new results. Measure disabled/enabled arms
on fresh seeds 2026110519 and 2026110520, 100,000 deals / 600,000 hands each, on C
and then S2. These seeds were not used for residual-size or position selection.
Report the actual full-table estimate and verify the outside-seat payoff is zero;
do not infer its CI by dividing separately reported seat intervals.

Before the C API child starts, register a scheduling-only barrier: it writes
its completed C API marker, then waits for these original-host C/S2 studies
before returning to its parent and releasing the unchanged 4M h2h gate. It
does not weaken any measurement gate or overlap a separate model job with h2h.

**Compose the API action repair with current main:** source archive
bed13e07336439538c3d42944c06bb86362288f7375b74bb608495b62d362fd1
starts at a51a377 and changes only the missing-distribution action, its replay
documentation/regression, and the always-run HTTP/batch test. The retained
probability reporting maps distributions after action selection and remains
unchanged. Run all required seven-package release tests/builds, including the
4,000 generated plus explicit fallback HTTP cases, with two workers. Rebuild
both current-main API harness arms, then exactly reproduce the completed old
C matrix primary, exact-grid and canonical scores/diagnostics at both registered
seeds (20,004 hands per arm). The full C/S2 control matrices remain mandatory;
the two repairs must compose without changing these sampled actions. This
serve-only change does not alter native blueprint/search/h2h behavior. Fresh
receipts: ~/fix/hardening-12/api-realstate-final-v1. No service is restarted.

**Next native hypothesis, before measurements:** when a non-all-in translates
nearest to an all-in, keep_sync preserves all-in flags by selecting a distant
smaller raise. Test an isolated opt-in `handoff_allin` (default false) that
explicitly hands off instead at that exact boundary. It reuses the existing
real-state fallback, preserves the current allin_snap/translation rules, true
shoves and ordinary on-menu actions, and does not relax synchronization. Start
from the unchanged iteration-8 mixed-passive source; compare that candidate
alone with the same candidate plus this handoff, using the same frozen attackers
and seeds. Add direct 95-BB-to-8.5-BB regression coverage, on-menu invariance,
and bench/search/training fallback parity. Required release checks precede
measurement. First measure the full residual-size grid and independent blind
probe on C while it is the only cached model; any fixed-probe gain remains
provisional until trained attacks, all external controls, final-model and
normal-strength gates pass. No defending default is changed by preparing it.

Preregister its initial C grid: residuals
1/2/5/10/25/50/100/200/500/1000/2000/3300/3400/4000/5000, both all-seat and
blind-only-with-outside-folding policies, 3,334 duplicate deals at seed
2026110521 per configuration/arm. Baseline is mixed-passive enabled; candidate
adds handoff_allin. First exactly replay both disabled/ enabled-mixed-passive
original probe statistics and diagnostics with handoff disabled, and require
the original fixed-target positive control. The primary is the already chosen
500-chip blind attack. Require disjoint improvement of both raw/corrected
primary CIs and no disjoint regression across the grid. Independently confirm
the primary and the grid's largest candidate mean (tie: smaller residual, then
all-seat) at seeds 2026110522 and 2026110523, 100k duplicate deals / 600k hands
per arm. Initial failure rejects this prototype before expensive adaptive gates;
initial success still requires the complete subsequent C/S2 gates. Immutable
source SHA256 12806d60b79818adc7fb91b9bb292e087ea8eb0ed25b7ece1af4a85cdbd90242;
source/build/fixed-attack receipts: ~/fix/hardening-13.

The residual-training coordinator was still waiting for its builds, with no
inference children, no command/result outputs, and only its registration/lock.
Preserve that waiting directory and stop only that coordinator; v2 keeps every
registered training/evaluation gate and adds explicit C/S2 phase-release barriers
for the native handoff experiment. No inference is interrupted or restarted.
Complete original C measurements before any additional large-archive evaluation;
do not switch to S2 until the additional C stage releases it. Likewise, complete
all additional S2 work before h2h starts. At most 11 training, four fixed-probe
and two composed API-replay workers run together; source builds use two workers
and are serialised. Model phase identity/metadata/boot gates remain mandatory.

**Validation assertion failure, investigated before continuing:** the first
blind-only C run stopped because its driver incorrectly required corrected
outside-seat AIVAT scores to be exactly zero. It had not begun training. The
four corrected means were +0.797/+0.399/+1.410/−1.957, with 95% half-widths
3.258/3.335/3.570/4.268; all include zero. AIVAT adds zero-mean corrections, so
an exactly zero raw strategy can have finite-sample corrected noise. Do not
force those estimates to zero or change the estimator.

Preserve that complete partial run and the resulting scheduler failure. Add
raw per-seat mean/CI/count diagnostics to the helper; retain every old corrected
and pooled raw field unchanged. Require exact raw zeros outside the blinds,
and use a separate 4-SE corrected-null diagnostic to halt for investigation,
without changing any reported 95% CI or defending-policy improvement gate.
V3 exactly reproduces the first run's complete existing statistics/diagnostics;
its new raw values and CIs are exactly zero in all four outside positions,
100,000 hands each. All other raw/corrected native references and both frozen
C binary references also reproduce exactly. There is no observed estimator
failure here; the erroneous assertion was in the new validation driver.

The continued full-table C probe (100k duplicate deals / 600k hands per arm):

| fresh evaluation seed | fixed main | mixed-passive candidate |
|---|---:|---:|
| 2026110519 | +4,551.9 ± 85.4 | +3,325.1 ± 81.1 |
| 2026110520 | +4,474.9 ± 85.3 | +3,258.5 ± 80.9 |

Raw respectively +4,534.4 ± 111.5 / +3,285.6 ± 110.6 and
+4,512.7 ± 111.6 / +3,294.7 ± 110.6. These independently sampled, fixed
blind-only attacks are stronger C attack bounds than the previous frozen 3M
all-seat attacker. Do not pool their estimates with that different strategy.
The candidate improves both samples significantly and still loses heavily;
final S2 confirmation remains queued. New 3M / 35-GB residual-menu training
started only after all new diagnostics/reference checks passed. Fresh driver
residual_study_v3.py preserves every seed, sample count and original four
training studies; only the invalid null assertion and added reporting differ.

The full C API scoring matrix completed all policy gates before the unrelated
null-assertion failure propagated through its later scheduling barrier. Its
completed measurements/marker remain preserved; that scheduler exit does not
count as a failed API poker comparison. Resume h2h only through a new controller
that explicitly verifies those completed gates and every remaining study.
No incomplete stage is silently marked passed.

Keep the two held native-handoff coordinators' waiting registrations/logs; they
had no build/inference children when stopped. V2 restarts them with unchanged
source/strategy/gates, a native-only helper carrying the same raw diagnostics,
and strict old-stat replay after excluding only newly added per-seat fields.
Use an independent build target, seven nice-10 workers and one test thread, so
the original 11-worker training plus two-worker API checks remain within the
22-worker budget. No live build target, service or model is modified.

**Narrow adaptive attacker registered before training:** reuse the task-owned
64-GiB worker for a different cache hypothesis, after stopping the earlier slow
large-menu attempt. It is intentionally woken for this study, boot
f2666d63-4a21-4fab-9f67-d09d8d77902d; the primary remains untouched. New full
buffered/direct C and frozen-learner hashes pass, and all four bucket files match
the original host exactly. Only trusted Rust is copied/run there, never Sombra
or credentials. Profile remains 16 CPU / 64 GiB; no larger machine is claimed.

The separate `pressure-v1` learner omits small pot-fraction raises, retaining
fold/check/call, real shoves and legal raises leaving 1/2/10/100/500/1000/3300
chips, with exact versioned action metadata. Its narrower target lookup paths
may fit cache. Add `--fold-outside-seats` so a frozen position-focused policy
can be evaluated through the actual six-seat match, folding elsewhere. The
always-run test verifies active-player preservation, legal exposed outside
fold/check behavior, mid-hand forks and exactly zero raw nonblind payouts.
The two menu definitions each pass exact archive reload/mismatch rejection.
Required seven-package release tests/builds pass before inference; immutable
source SHA256 f182bb8c50f795eec8fe19fe56ecfc23d9579c76ee7c89543c6497310f88634c.

First require exact old C frozen-reference estimates/miss rate, normalizing
only the two added disabled option labels. Then train 3M iterations / 5 GB,
12 nice-10 workers, seats 0/1, seed 26, against mixed-passive plus the experimental
all-in handoff. That target is fixed before any native-handoff poker result;
this is an independent adaptive attack, not evidence the defender is ready.
At five minutes require the latest cumulative training rate to be >=100 it/s;
otherwise terminate only this owned process group, preserve resource/progress
logs, and report an incomplete resource experiment without EV or usable weights.

On completion, hash the native archive with buffered/direct reads and require
exact focused enabled reload. Freeze it for main / mixed-passive / combined
target variants, both average/argmax extraction, discovery 2026110527 and
independent confirmation 2026110528, 100k duplicate deals / 600k hands per arm.
These are actual full-table scores with outside-position folding; retain all
raw/corrected estimates, original/candidate regression comparisons, miss rates
and store saturation. Negative finite-learner scores prove no robustness claim.
Finish/review all C work before explicit release-c.ok allows the worker to
hash S2 in full, replay its frozen reference and repeat the complete frozen
matrix. Receipts: /work/sharingan-model-validation/hardening-14. No deployment.

**Current-main API suite stopped on a pre-existing statistical fixture:** the
full v1 release run reaches pluribus::eval_pipeline, where the 60k-versus-200
iteration toy comparison produces +167.54 ± 194.97 in the mixed table. Its
asserted significant win is inconclusive. The API decision repair is not used
by this native test; the fixture trains Hogwild models and assumes their
finite-sample strength difference. Preserve the failed suite instead of
retrying until the random training outcome passes. The other two pipeline
tests passed; the later API suites had not run in this stopped command.

Replace only that detection fixture with persisted, compacted pure strategies:
aggressive seats shove/call, the other seats fold to bets. Enumerate every
reachable preflop public history across the evaluated compositions and store
the action for every one of the 169 hole-card classes. All pots go to the
aggressive group, so its group gain is exactly the other group's posted blinds.
For the actual balanced seatings, raw scores must be +250 / +1,250 / −250
mbb/game in the three formats, independent of cards. Keep the significant
positive corrected-edge assertion, all 3,000 deals, and add the known raw-score
and zero-variance assertions; do not widen a CI or lower a strength threshold.
Check this fixture with 1/2/4 Rayon workers, then repeat the complete required
release suite/builds and current-main API replays. Correct the evaluation docs
to describe the existing eight mixed seatings (block rotations plus two
alternating patterns); no h2h engine, production policy or historical method
is changed. V2 archive SHA256
46230576424c70e93122b8647a469830b7622c5c41687d24424924700b59c4d6;
fresh receipts: ~/fix/hardening-12/api-realstate-final-v2. API retention remains
pending this full gate.

**Performance experiment registered before timings:** iteration 3's per-deal
cache failed end-to-end repeatability and remains rejected. Test a different,
bounded design: immutable tables of the exact existing 400-sample preflop
equities for every 1,326 actual hole-card combinations and 1–5 opponents.
Keep actual cards rather than the 169 rank classes because the deterministic
sampling seed uses the card mask. Model constructors initialize complete tables
before serving hands, independent of the private hand; direct fallback callers
that skip warmup retain the old calculation. Postflop uses the unchanged code.
The five tables occupy about 108 kB. There is no policy formula, sampling budget,
translation or random-stream change, and startup time must be included in timing.

First require bit-exact equality for all 6,630 valid preflop keys, reversed-card
queries, concurrent initialization and postflop bypass. Run all seven required
release packages, using the same new known-payoff H2H fixture. Against verified
C require the exact 100k-deal frozen 1M reference. Then require two old and two
new single-thread 1k-iteration runs at seed 30 to have identical complete native
learner-file hashes, infosets/slots/nodes/store-full/miss counts, target diagnostics
and evaluation estimates. Any difference stops the experiment before a speed
claim. This is stricter than iteration 3's already-failed comparison.

If exactness passes, time old/new subprocesses at seeds 31–35, alternating order,
one thread, 1 GB, pressure-v1 seats 0/1, fixed combined experimental target,
200 evaluation deals at seed 2026110529, saving every archive. Include model
open, cold full-table initialization, training, evaluation and file writing in
whole-process wall time. Require exact paired archive/statistic identity at
each seed. Report the geometric paired speedup with a 95% Student-t interval
over log ratios (df 4, t=2.776445105); its lower bound must exceed 1.20 to justify
retention. This is a worker/workload performance metric, never a poker EV gain.
Require final S2 frozen-reference equality and current-main source composition
before a cache-only commit. C-only jobs stop before that worker's explicit
model-phase release. Four build/reference workers plus its 12-worker learner
fit its 16 CPUs; timings use one worker after the build. Source SHA256
3d58d31150e3f565be124c08283be5279c91cf90e070f6e5f951104b252af20c;
receipts: /work/sharingan-model-validation/hardening-15. No deployment.

**Snap-restricted native handoff: initial fixed-attack results.** Both independent
C holdouts pass the registered primary raw/corrected improvement gate, with no
significant regression in the discovery/holdout matrix. These are 100k duplicate
deals / 600k actual full-table hands per arm, folding outside the blinds:

| seed / chips left | mixed-passive only | plus snap-restricted handoff |
|---|---:|---:|
| 2026110522 / 500 | +3,325.4 ± 81.0 | −4,517.5 ± 46.1 |
| 2026110523 / 500 | +3,268.5 ± 80.8 | −4,528.6 ± 46.2 |
| 2026110522 / 4000 | +1,546.1 ± 63.2 | +1,545.7 ± 63.2 |
| 2026110523 / 4000 | +1,509.6 ± 62.9 | +1,508.9 ± 62.9 |

The 40-BB residual remains significantly profitable. Initial fixed-attack
success is not retention: frozen/adaptive/SDK/LBR/S2/normal-strength checks
remain mandatory. All flags stay opt-in in isolated worktrees.

**Nearest-all-in follow-up, registered before measurements.** The existing
0.5 snap filter excludes the shove from the translation bracket for a 60-BB
bet. Test a separate default-false flag that computes the full deterministic
nearest bracket solely for deciding whether to hand off explicitly. Accepted
translation, true shoves, legal on-menu play and random streams are unchanged.
`Bracket::Exact` also denotes menu clamping, so it cannot identify exact on-menu
bets; calculate the full bracket for every opponent raise and test that boundary.
Mark this new explicit handoff off-tree, leaving the old disabled experiment's
metadata untouched. Add boundary, on-menu and bench/search/training parity tests,
plus independently selectable h2h flags. No live environment is changed.

Source SHA256 bf87c38f84d65695a5f63febc578798faba91254922379c75e57fd528fab2066;
remote seven-package release tests/builds precede inference. Exactly reproduce
the completed 2026110523 4000-chip snap-handoff holdout with the new flag off,
including all raw/corrected per-seat values and diagnostics. Baseline is
mixed-passive plus snap-restricted handoff; candidate additionally enables the
new nearest-all-in guard. Fresh discovery seed 2026110540, 3,334 duplicate deals
per arm, both all-seat and blind-only policies, residuals
1/2/5/10/25/50/100/200/500/1000/2000/3300/3400/4000/5000/6000/7000/8000/
8504/8505/8506/8507/9000/9150. Primary: 4000 chips, blinds only. Select the
largest candidate discovery mean (ties: smaller residual, then all-seat), then
confirm it and the primary at fresh seeds 2026110541/2026110542, 100k duplicate
deals per arm. Require disjoint raw/corrected primary improvement at discovery
and both holdouts, reject any significant regression across all sampled pairs.
Outside-seat raw scores must be exactly zero; corrected-null 4-SE diagnostics
remain separate from the reported 95% intervals. Initial failure rejects this
prototype; success still requires full adaptive/frozen/control/final-model and
normal-strength gates. Receipts: ~/fix/hardening-16; no automatic promotion.

**Current-main API release checks:** v2 completes 185 reported passes across
40 suites, zero failed/ignored. The same 17 fixture-dependent tests return early;
168 tests actually execute, including the authenticated 25 single + 25 batch
cases and all 4,000 generated HTTP cases plus explicit scaled-terminal control.
The known-payoff fixture passes independently at 1/2/4 workers with exact raw
+250/+1,250/−250 and zero variance. The post-test mandatory-name checker falsely
rejected a module-qualified Rust test name; preserve failure.json and correct
only its matching expression. The continuation then stopped before its first
build because its package-selector variable was missing; preserve that separate
failure too. V3 verifies all unchanged source hashes, complete test summaries
and mandatory names before continuing the unfinished build/replay stages.
No test or poker threshold is relaxed; API retention still awaits those stages.

**Narrow pressure learner completed:** 3M iterations in 3,209.50 seconds,
83,987,788 infosets / 475,390,968 slots, store not full. Complete native archive
buffered/direct identity and exact enabled reload pass; unseen evaluation rate
is 1.72% in the focused discovery sample. Initial actual full-table average
extraction at discovery 2026110527 scores +55.09 ± 32.91 against main,
−52.05 ± 32.05 against mixed-passive, and −239.47 ± 30.88 against combined
snap-handoff. The independent main holdout is +73.20 ± 33.31. Remaining holdout,
argmax and S2 measurements are pending; this finite learner omits the newly
identified 4000-chip residual and does not bound all remaining attacks.

The first nearest-handoff build stops before poker inference: its expanded
bench/search parity test moved a shared `Arc` in the first loop iteration and
then reused it. Preserve the v1 failed build and its waiting-study failure.
Clone that test-only Arc, leaving every production source and measurement gate
unchanged; fresh v2 build/study directories repeat all required release checks.
Corrected source SHA256 953528d4cdb37df82652090ed8b0a4f7d97a1cb3e5028e130f8933c0718b5651.

**API action repair kept after all gates.** The current-main composed build
and all twelve 20,004-hand primary/boundary/canonical replays pass: every old
attacker statistic and target diagnostic is exact on both registered seeds.
Required release tests/builds pass as reported above. The full independent
C/S2 API matrices complete their primary improvement and all raw/corrected
control gates with zero API errors. Main changes only the absent-distribution
API branch to the existing real-state strength fallback; no native blueprint,
search policy, translation, model weights or h2h action changes. Retain the
always-run replay/HTTP tests, the deterministic known-payoff evaluation fixture
and accurate eight-seating documentation. Experimental native flags and learner
menus stay outside main. No live service, public result or deployment changes.

| model / seed | original API attacker mbb/game | repaired API |
|---|---:|---:|
| verified C / 2026110410 | +14,283.3 ± 910.5 | −2,391.2 ± 453.9 |
| verified C / 2026110411 | +14,850.8 ± 934.5 | −2,854.3 ± 474.4 |
| SHARINGAN-2 / 2026110410 | +13,900.5 ± 888.7 | −2,325.7 ± 452.0 |
| SHARINGAN-2 / 2026110411 | +14,532.0 ± 918.7 | −2,757.5 ± 472.1 |

This API scaling result does not fix the separate native residual-size leak or
establish robustness. Full exact commands, hashes, raw CIs and all controls:
~/fix/hardening-11/api-{c-v1,s2-v3}, with composed current-main receipts under
~/fix/hardening-12/api-realstate-final-v2. Original checker/continuation failures
remain preserved alongside their explicit recovery receipts.

API action repair committed and pushed as 899e09b; source and all measurements
above remain private and undeployed. The nearest-handoff v2 recovery wrapper
itself rejected Rust 1.99's reformulated E0382 wording before compilation; v3
checks the actual error code, test location and moved-value note instead.
The corrected source remains 953528d4cdb37df82652090ed8b0a4f7d97a1cb3e5028e130f8933c0718b5651;
all previous failed build/study receipts remain preserved. No poker run has
been repeated or reselected for this candidate.

**Cache prototype rejected after exactness passed.** All 6,630 keys, complete
single-thread archive hashes/counters/estimates, old/new repeatability and each
of five paired seeds are exact. End-to-end paired speedups are
1.9942/0.6280/1.6341/0.6757/1.5977; geometric mean 1.1718×, paired-log Student-t
95% interval [0.5967, 2.3012]. Startup is included. The preregistered lower-bound
>1.20 gate fails, so retain no cache production code; skip its unnecessary S2
retention check and preserve every timing/archive receipt. No speed or EV gain
is claimed. The separate pressure study has completed all twelve C cells with
no significant raw/corrected regression; review finds no C inference child, and
explicitly releases only that worker to S2 for its full identity/reference and
frozen-matrix checks. The original Ubuntu host remains in its separate C phase.

| frozen pressure policy / seed | main C | mixed-passive | plus snap handoff |
|---|---:|---:|---:|
| average / 2026110527 | +55.09 ± 32.91 | −52.05 ± 32.05 | −239.47 ± 30.88 |
| average / 2026110528 | +73.20 ± 33.31 | −36.27 ± 32.35 | −213.79 ± 31.20 |
| argmax / 2026110527 | +30.68 ± 28.18 | −42.75 ± 27.51 | −97.63 ± 26.90 |
| argmax / 2026110528 | +51.58 ± 28.41 | −24.66 ± 27.68 | −83.60 ± 26.86 |

Each cell: 100k duplicate deals / 600k actual full-table hands, learned policy in
blinds and folds elsewhere. Focused training evaluation is a different estimand
(−718 ± 92 across only the two learned positions), not pooled with this table.
All full-table raw estimates/CIs and miss rates remain in the private receipts.
These finite adaptive bounds do not negate the profitable separate 40-BB probe.

**Broader adaptive menu prepared before training:** pressure-boundary-v1 retains
fold/check/call, shoves and residual chips
1/2/10/100/500/1000/3300/4000/5000/8000/8505/8506/9150. It includes the known
40-BB leak, the exact nearest-all-in cutoff on both sides, and the ordinary
8.5-BB menu raise. Existing pressure-v1 and residual-v1 action lists/manifests
remain byte-identical. Test all three archive definitions/rejection, legal
sorted unique actions/caps across 2–6 players, and the real six-seat outside-fold
wrapper. Compose only the retained API repair plus unchanged experimental native
flags; the rejected cache is excluded. Compilation uses four original-host
workers on CPUs 18–21, alongside eleven training and seven native-test workers
on CPUs 0–17 (at most 22 computing CPUs total). No boundary training or quality
result is claimed yet. Source SHA256 9741cb7871bd58d9006b6aaee74d8e6aecb8677a7eb42d73a912e7d789fed797;
receipts: ~/fix/hardening-17/boundary-build-v1.

The first boundary-menu builder stops at cargo fetch because its clean HOME
has no configured Rust toolchain. Preserve that failure; v2 points CARGO_HOME
and RUSTUP_HOME at the existing Ubuntu installation and fetches offline. Source,
flags, test/gate definitions and seeds are unchanged; no training or poker
measurement occurred. Fresh receipts: ~/fix/hardening-17/boundary-build-v2.

The nearest v3 suite then reaches the second expanded parity fixture and finds
the same test-only Arc move in exploit::abr. Clone that bucketer too; no production
policy code changes. Full release tests/builds remain required. Fresh nearest
v4 source SHA256 cd05f918c9b475e88a6510b7dabc063cad63ac77f2b80dd34585a900da0f5a72,
receipts: ~/fix/hardening-16/nearest-{build,study}-v4. The boundary v2 builder's
all-platform offline fetch requests an unused wasm dependency and stops before
compilation; remove that redundant fetch, retaining fully offline mandatory
native release tests/builds. Boundary v3 includes the same test-only Arc clone;
source SHA256 a725148784cf6a5d1463472cb81625510ff04e6ae3ad18f2c285410e21b73934. All earlier failure logs remain.

**Private real-worker SDK follow-up registered before running:** after the
nearest fixed-attack stage and boundary-menu release builds, exactly replay the
original native frozen 1M C reference with every new option disabled, including
all estimates/miss rate/target diagnostics. Require Sombra's original pre-sync
positive control lower CI >+1000. Then run the original nine SDK controls/probes
(quickstart, TAG, Sombra and all six probes) through `sharingan eval --local`,
track sharingan-1 unchanged, 20,004 hands, four lanes, seed ab-20004 per arm.
Variants: main, mixed-passive, snap handoff, and nearest guard only if its fresh
registered primary/grid/holdouts pass. Compare every later variant with earlier
variants, identical deal digests, reject a disjoint corrected-CI regression;
require zero crashes/forfeits/illegal actions/timeouts or bridge tracebacks.
The worker exports raw duplicate means but no raw CI, so retain those means
without claiming a raw significance gate. Native frozen/adaptive/LBR raw and
corrected gates, final S2 and normal strength remain mandatory for retention.

Run only as ubuntu/nice10, no-new-privs, empty HOME and whitelisted credential-free
environment. The CLI uses local pipes, no hosted endpoint, submission or account.
Use CPUs 11–21 only after both other experiment builds/initial probes finish,
alongside eleven original-host training workers on CPUs 0–10. Initial directory:
~/fix/hardening-16/e2e-controls-c-v1; no overall C phase-release marker is written
by this SDK-only stage. No public row or live service is touched.

**Additional native SDK/builtin controls registered:** after the official SDK
worker stage completes, compile an isolated native CompactPolicy harness with
the unchanged SDK bridge/adapter. First exactly reproduce the original all-seat
500-chip-residual C sample (seed 2026110516), including all existing raw/corrected
statistics and target diagnostics, before accepting any new SDK score. Report
this adapter's preserved per-hand memory/history as a distinct attack from the
original worker; do not merge their Sombra scores. The target exposes its actual
native probabilities, and SDK bot actions stay unexposed.

Fresh seed 2026110545, 3,334 duplicate deals / 20,004 hands per arm: original nine
SDK bots plus nit/TAG/LAG/maniac/station/random, main/mixed-passive/snap and nearest
only if its fixed stage passed. Compare every later variant against earlier
ones; reject disjoint raw or corrected 95% regression. Abort on reference mismatch,
SDK errors/timeouts/substitution/traceback, model metadata/boot changes or C phase
release. Four match workers plus at most four SDK workers use CPUs 11–18, after
the other SDK stage, alongside original 11-worker training. No live pipeline is
used or changed; all other frozen/adaptive/LBR/S2/strength gates remain mandatory.
Helper SHA256 d8958f3638941d112b56c1756a5cc8f7b0137437448ac46f6df08be2ae2cb815;
receipts: ~/fix/hardening-16/native-sdk-{build,controls}-v1.

**Boundary expectation failure investigated before poker inference.** The nearest
v4 release suite fails its new 1495-chip expectation. The production guard is
using the existing pseudo-harmonic formula correctly; the test assumed a plain
harmonic midpoint. For pot fractions a=3 and b=39.6, p_lower=0.5 occurs at
(2ab+a+b)/(2+a+b), corresponding to raise-to 1670.6278 chips. Assert the analytic
formula and both integer sides (1670 follows / 1671 hands off); 1494 and 1495 both
follow. No defending production code or gate threshold is changed, and no poker
measurement has run for this candidate. Preserve the failed suite and dependent
waiting-stage failures. Run the targeted boundary test before the full suite to
catch such fixture errors promptly. V5 source SHA256
34749d8077575d7352e9a66c76c3f2abb15a0491102ec9f05e2098ed6b1684cf;
fresh receipts: ~/fix/hardening-16/nearest-{build,study}-v5.

Before any native grid measurement, add actual 8329/8330-chip residual boundaries
to the original registered size grid, retaining every original case and all
seeds/counts/selection/retention gates. Before any adaptive training, give the
broader menu a new pressure-boundary-v2 identity with the same two added sizes
(15 residual sizes, 18 root actions); old pressure-v1/residual-v1 remain exact.
The earlier boundary-v1 was compiled/tested only, never trained for a quality
result. New source SHA256 bc4ed6932aa03742e20c68f4e1dd59aed4991451ffe0a739eb992bfe17bb3bfd;
fresh builder: ~/fix/hardening-17/boundary-build-v4. SDK/control v2 queues use these
new completed-build prerequisites with unchanged hands/seeds/policy gates.

**Cargo artifact reuse failure; new policy measurements held.** The corrected
boundary source is byte-identical on both build roots (SHA256
32fa76b2fdab551ee9c894267b0789ae59905d8c609b991b99afded000a87e8a).
The nearest build compiles and passes its corrected analytic/1670/1671 test,
but the boundary build reports Finished in 0.15 seconds and executes the previous
1495-chip test binary, including its old assertion line. A shared Cargo target
across extracted source roots is not a valid provenance gate. Stop only owned
experimental build/queues before candidate poker inference, preserve every log,
and use a new target directory per exact source archive and separate targets
for baseline/candidate roots. Source hashes alone do not prove binary provenance.

Independently revalidate committed API 899e09b from unchanged archive
46230576424c70e93122b8647a469830b7622c5c41687d24424924700b59c4d6 with fresh,
separate targets, all seven release packages, mandatory HTTP/fuzz/known-payoff
checks and all twelve exact C poker replays. Its earlier primary improvements
were directly witnessed, but require the clean current-main source audit too.
Fresh receipts: ~/fix/hardening-18/api-realstate-clean-v1. Rebuild nearest source
34749d8077575d7352e9a66c76c3f2abb15a0491102ec9f05e2098ed6b1684cf in
~/fix/hardening-16/nearest-clean-build-v1 with its own new target, targeted boundary
then the full suite, before a fresh unchanged grid/holdout stage. Keep the
separate H12 immutable training and unique-target PTXP H14 model checks running;
no service, live build, model or /srv file is touched. CPUs: 11 training + 7 native
build + 4 clean API build, at most 22. No new native defending change is retained.

The clean nearest target compiles and passes the corrected boundary unit test.
The full suite is running. The separate boundary clean builder waits for the
clean API audit, nearest build and frozen-learner transfer before starting its
own fresh target (seven workers); no old target is copied or cleaned in place.
Fresh SDK queues retain the exact registered bots, seeds, counts and gates and
wait for those clean artifacts. Directories: hardening-17/boundary-clean-build-v1,
hardening-16/e2e-controls-c-clean-v1 and native-sdk-{clean-build,controls-c-clean}-v1.
Archive sources and hypotheses are unchanged. All old interrupted/failed queues
remain preserved, and no native candidate score has been accepted from them.

Stale-build evidence is preserved under hardening-11/cargo-stale-evidence-v1:
corrected assertion is at source line 1089, while the reused binary reports old
line 1083; the dependency file uses relative source paths. Record its exact binary
hash and failed log rather than attributing an unverified upstream cause. The
new isolated targets cannot contain that old artifact. Clean API known-payoff
fixture checks at 1/2/4 workers pass; full release suites remain in progress.

**Additional full native C gates registered before measurements.** After both
clean SDK matrices, screen out handoff candidates with any significant regression;
nearest additionally requires its clean fixed-attack primary/grid/holdouts. Keep
main and mixed-passive as controls. For each surviving handoff, hash each native
learner with buffered/direct reads against its original manifest, then run the
old 1M / 3M / price-trained 1M / odds-trained 1M archives, average and argmax,
fresh seeds 2026110548/2026110549, 100k duplicate deals / 600k hands per cell.
Compare every later variant with earlier variants, retaining raw/corrected CIs.

Transfer only the task-created pressure archive (native SHA aaca3fb65243987442d24e75b24aa4f710c3b1f2575613a4cb518d90ffaf335e)
through the Mac, without credentials. Verify compressed/native hashes and all
four identical bucket files; explicitly relocate only the bucket path in a copied
manifest. Exactly replay all twelve old C pressure cells across hosts before
interpreting the new nearest arm at the same frozen seeds/extractions. Preserve
the original evidence bundle SHA bf0a44863636ea7ab94b784463779b357406f2ccf999e6bafe7a14faf69f8ab9.

Run fresh standard weak/strong profile LBR for controls and surviving handoffs,
20k deals, CLI seed 5, 300 Monte Carlo samples, default/richer menu. LBR reports
actual raw returns; do not label these AIVAT-corrected. As each of the four newly
registered H12 3M learners finishes and passes archive reload, hash it anew,
exactly replay its original first mixed-passive average sample, reuse its completed
main/odds baseline cells explicitly, then evaluate surviving handoffs on both
original seeds 2026110513/2026110514 and both extractions. Those focused-seat
estimates remain conditional and are not pooled with full-table scores.

Any disjoint 95% raw/corrected regression rejects that candidate; LBR uses its
actual raw-only CI. Propagate any main-to-odds regression to the existing old
candidate rejection flag so normal-strength work cannot silently promote it.
Complete/screen the C stage before releasing the original host's model cache;
mark S2 skipped only if every additional handoff is rejected, explicitly saying
no policy was kept. C survivors still require full S2 and normal-strength gates.
Four nice-10 workers use CPUs 11–14 after the SDK stages, alongside eleven original
training/evaluation workers. Driver/receipts: hardening-13/native-full-c-clean-v1.
No deployment or automatic policy retention occurs.

**Clean audit and native fixed-attack results.** The isolated current-main API
audit completes 185 reported passes / 40 suites (zero failed/ignored; the same
17 fixture-dependent early returns leave 168 actually executed tests), all
release builds, and all twelve exact primary/boundary/canonical C replays.
Both A/B arms have separate fresh targets. The committed API repair's original
C/S2 improvements are reproduced, not replaced or reselected.

The clean nearest-all-in guard passes its full required release suite/builds,
exactly replays the old snap-only reference, and passes all registered initial
C fixed-attack gates. All 52 candidate discovery configurations' corrected upper
95% endpoints are negative; the largest is −1,099.1. The selected boundary is
8330 chips left (raise-to 1670), blind-only. Independent holdouts, each 100k
duplicate deals / 600k hands per arm:

| seed / chips left | snap-restricted handoff | plus nearest guard |
|---|---:|---:|
| 2026110541 / 4000 | +1,515.1 ± 63.3 | −4,854.5 ± 51.3 |
| 2026110542 / 4000 | +1,487.0 ± 63.3 | −4,817.5 ± 51.5 |
| 2026110541 / 8330 | −1,197.4 ± 51.7 | −1,430.7 ± 51.9 |
| 2026110542 / 8330 | −1,271.7 ± 52.0 | −1,490.9 ± 52.1 |

Primary raw holdouts respectively +1,515.8 ± 92.8 → −4,874.8 ± 108.2 and
+1,491.7 ± 92.8 → −4,766.8 ± 107.4; raw/corrected primary intervals are disjoint,
with no registered significant regression. This is an initial C result, not
policy retention or final-model robustness. SDK/frozen/adaptive/LBR/S2/normal
strength gates remain. The transferred learner's compressed/native hashes and
all four bucket hashes pass on the original host; only its copied manifest's
bucket path changes. Delete only the task-created 1.3-GB Mac transfer temporary;
identical files/receipts remain on both workers.

**New attacker-extraction hypothesis, before measurements:** unseen learner
infosets currently play uniformly during training and check/call during frozen
evaluation. That documented mismatch may weaken finite attacks. Add evaluation-only
passive/uniform/strength choices, default passive unchanged; learned-node average
and argmax probabilities and native weights stay unchanged. Strength uses the
existing real-state heuristic and can emit legal off-menu raises. Reject invalid
extraction/missing-mode enum values before model I/O and report the actual two
selected modes in each result. This changes no house policy or training update.

Tests cover royal-flush/weak-hand behavior with real chips/cards, legal exposed
uniform probabilities, fork preservation, bit-exact learned distributions for
both extractions under every missing mode, actual CLI play rather than labels,
and default-versus-explicit-passive equality. Run all seven required release
packages from a new source-bound target before any poker comparison. Source
18b9f5ea2861a18bd0a12f79af69b72a0b533d90c1438c8b63a6c16d357336f9
changes only exploit::abr/cli relative to the current native prototype. Three
nice-10 build/evaluation workers use CPUs 19–21; 11 original workers plus at most
eight SDK workers plus these three stay within 22. Source/receipts:
hardening-19/learner-fallback-build-v1. No attacker feature is kept yet.

The clean boundary source completes 205 reported passes across 40 suites,
zero failed/ignored (the existing 17 fixture early returns remain). Its final
mandatory-name checker falsely rejects the archive test because --nocapture
prints its JSON and intentional caught panic between the name and final `ok`.
Preserve the failure; verify every unchanged source hash, all passing summaries
and module-qualified mandatory entries, then recheck that deterministic test
with captured output before continuing only unfinished release builds. No poker
score or retention gate is relaxed. Apply the same explicit recovery if the
running new learner-mode suite hits the same checker limitation; actual test
failures stop it. Stable source-bound targets are reused only for the same
unchanged snapshot, never another root or A/B arm.

**Next frozen-extraction experiment registered before scoring (2026-11-06).**
Use the four certified old 1M/3M/adaptive/odds archives, both average and argmax,
and passive/uniform/strength missing-node modes against the nearest guard.
Discovery: seed 2026110660, 3,334 deals per cell (24 cells). Select the nondefault
mode with the highest corrected attacker mean; deterministic ties prefer the
existing archive order, average, then uniform. Proceed only if it exceeds the
best passive discovery mean. Freeze that selection before two held-out seeds
2026110661/662, 100,000 duplicate deals each. Compare selected extraction,
same-archive passive extraction, and best passive profile, across all four
main/odds/snap/nearest defenders. A feature gain requires disjoint raw and
corrected intervals against both passive controls on both nearest holdouts.
This is a finite attacker comparison, never an exploitability upper bound.
Record all defender regressions and import them into the native retention gate.
Fresh source-bound build, default exact historical replay, per-run model/archive
identity checks, and final S2/feature-only composition remain mandatory.
Commands: `nice -n 10 python3 -I -u hardening-11/learner_fallback_study_c_v1.py`;
receipts: hardening-19/learner-fallback-study-c-v1. Three workers on CPUs 19–21.

The captured boundary archive recheck and remaining release builds complete;
source bc4ed6… is unchanged. Restart the previously failed prerequisite queues
in fresh `e2e-controls-c-clean-v2`, `native-sdk-clean-build-v2`, and
`native-sdk-controls-c-clean-v2` directories, preserving their v1 failures.
The additional C coordinator becomes `native-full-c-clean-v2` and explicitly
waits for/imports the new extraction study before releasing the C cache phase.
No test, confidence rule, policy default, or historical score changes.

**Secondary-worker interruption investigated.** Its boot changed from f2666d63…
to e68db1f4… while hashing S2. No S2 poker cell or completed checksum exists.
The 3M attacker and all twelve completed C measurements survive and were already
independently transferred/verified. Read-only diagnostics show autopilot disabled,
the task worker running with its volume saved, and no remaining study process;
the restart cause is unestablished. Resume only in a new directory, validating
binary/source, learner/buckets, and full S2 hashes under the new boot before any
inference. Do not repeat/select C scores or retrain the preserved attacker.
The original large-memory host retains the same boot and its training continues.

**Stronger final-model adaptive attacker registered before training.** After
the recovered H14 S2 matrix finishes, use the clean boundary executable/source
bc4ed6… on the task-owned secondary worker. Train `pressure-boundary-v2` for
3,000,000 iterations, 5-GB store, blind seats 0/1, seed 68, against experimental
mixed-passive + snap + nearest handoff. Its 15 residual sizes include the known
40-BB exploit and both sides of the exact 1670/1671 translation cutoff; the older
seven-size pressure menu/archive remains unchanged. Require at least 100 it/s
at five minutes or stop with no accepted model/score. No retry or menu tuning
based on held-out results. Freeze the archive and verify native/direct hashes,
then exact-reload its focused training evaluation (seed 2026110672).

Held-out seeds 2026110673/674, 100,000 duplicate deals each, average and argmax,
all four main/odds/snap/nearest defenders, full-table blind policy with folds in
other seats. Keep every raw/corrected CI, miss rate and store-full diagnostic;
any disjoint defender regression rejects retention. Frozen 1M S2 replay must be
exact before training, under the fresh-boot full S2 input certificate. Only 12
nice-10 workers CPU0–11; serialized after H14, no competing model on this worker.
The target remains experimental. This study cannot establish unexploitable play
or promote a defender. Command: `nice -n 10 python3 -I -u
hardening-14/boundary_s2_study_v1.py`; receipts: hardening-14/boundary-s2-study-v1.

**Final-model continuation gates registered before measurement.** On the
original host, serialize S2 SDK/extraction work after all H12 S2 measurements,
while its coordinator waits for the additional S2 phase release. Use the same
certified boundary/SDK executables and independently exact S2 frozen/probe
replays; no changed source or shared Cargo target is introduced. Repeat the nine
original bots through the actual private worker (seed ab-20004, 20,004 hands),
then the 15 native SDK/builtin profiles (seed 2026110545, 20,004 hands), all four
main/odds/snap/nearest arms and the existing raw/corrected gates. The worker
exports only a raw mean, so no raw confidence interval is invented for it.

Independently discover S2 unseen-node extraction profiles at seed 2026110675,
3,334 deals per cell across the same four frozen archives/two extractions/three
missing modes. Freeze selection before seeds 2026110676/677, 100,000 duplicate
deals per arm, comparing selected/same-profile-passive/best-passive against all
four defenders. Require disjoint raw and corrected gains over both passive
controls at both seeds to call this an S2 attacker feature gain; independent C
gain and clean feature-only composition remain required to retain its code.
Import every actual defender regression before native retention/phase release.
Eight SDK workers plus three extraction workers stay below 22, and run only S2.
Drivers: native_e2e_controls_s2_clean_v1.py,
reuse_native_sdk_s2_clean_v1.py, native_sdk_controls_s2_clean_v1.py,
learner_fallback_study_s2_v1.py; fresh receipts in hardening-16/*s2-clean-v1 and
hardening-19/learner-fallback-study-s2-v1. No deployment or automatic retention.

**Private-worker checkpoint, not a completed native gate.** Fresh C positive
control gives original Sombra +5,484.0 ± 511.5 with pre-sync-fix, then current main
−152.1 ± 129.5 (20,004 hands, seed ab-20004). The old frozen 1M replay is exact.
Quickstart main −596.4 ± 135.2 / odds −628.4 ± 136.7 / snap and nearest both
−646.6 ± 138.0; TAG main −465.6 ± 118.5 / odds −463.8 ± 118.3 / snap and nearest
about −465.6 ± 116.7. These completed controls show no disjoint regression;
Sombra's remaining arms and probes are pending. Original submission process
history makes this a distinct fresh private sample, not an exact reproduction of
an old board row. All numbers are the official worker's corrected estimates.
The new learner-mode suite/release build also completes 209 reported passes /
40 suites, zero failed/ignored (192 executed after the same 17 fixture returns),
with documented captured-output checker recovery. No feature code is retained.

**Cache scheduling and final-model retention follow-up.** Before any new C
coordinator scoring, replace its idle v2 process with v3 (no children/commands
existed; preserve scheduler-replaced.json). V3 releases only C. The new sole S2
coordinator waits for all original S2 measurements, both final-model SDK matrices,
and final-model extraction measurements, including when no handoff survives C.
This prevents optional S2 work from racing an early h2h cache transition; no
poker result or threshold changes.

For C-qualified candidates not rejected by S2 SDK/extraction, final confirmation
uses all four old frozen archives at seeds 2026110548/549, both extractions,
100k deals; the transferred pressure archive at 2026110527/528, both extractions,
100k deals; H12's four new archives at 2026110513/514, both extractions and their
original conditional trained-seat estimates; and fresh 20k-deal weak/strong LBR,
seed5, 300 samples. All gates use actual raw and corrected CIs, except raw-only
LBR. Exact original S2 disabled/reference replays are mandatory. Previously fully
hashed learner inputs require stable stat and manifest identity before reuse.

Fixed final-model blind attacks: 500/4000/8330 chips left, seeds 2026110541/542,
100k duplicate deals, all four variants. Snap must significantly improve over
odds at left500, nearest over snap at left4000, in raw and corrected estimates
at both seeds; any disjoint defender regression rejects retention. Eleven
nice-10 workers CPU0–10 after all other S2 scoring, so no competing model phase.
Driver: native_full_s2_clean_v1.py; receipts: hardening-13/native-full-s2-clean-v1.
It records survivors and releases scheduling only. Independent stronger boundary
learner review, full normal-strength and final clean main composition remain
required; no automatic source retention or deployment.

**Normal-strength protocol registered before h2h.** Only C/S2 attack survivors,
also accepted against the independent new 3M boundary attacker, proceed. Transfer
only the completed secondary worker's sixteen held-out JSON results/commands and
input/source/reload/reference proofs; verify every checksum, runtime option,
seed/menu/seating and independently recompute every raw/corrected CI gate before
import.done. A resource-aborted or incomplete attacker is not a passed gate.

Serialize after the older odds-only h2h controller. Its explicitly documented
odds-only candidate rejection remains valid; unexpected I/O/input/test/baseline
failures stop the new controller. For each qualified combined candidate, use
200-deal mechanical smoke and then 4,000,000 deals per format, seed1, all three
formats against unchanged verified C. First run a fresh current-main S2/C
baseline and require its mixed-format interval to overlap +5.09 ± 1.16. Reject
any disjoint raw/corrected normal-strength regression, reversing the sign for
the mirror five-vs-one format. Only this sole 12-worker nice-10 process may map
both models on the original host. No additional scoring child remains at release.
Receipts: hardening-14/boundary-s2-{export,import}-v1,
hardening-13/native-h2h-final-v1; drivers export_boundary_s2_v1.py,
import_boundary_s2_v1.py, native_h2h_final_v1.py. Final clean main composition and
full tests/replays remain mandatory after all gates; nothing deploys itself.

**Complete real private-worker C control matrix.** All nine profiles × four
arms finish, 20,004 hands/cell, seed ab-20004, matching per-profile deal digests,
zero forfeit/crash/timeout/illegal-action counts and no disjoint corrected-CI
regression against any earlier arm. The independently replayed frozen control
and original Sombra pre-fix positive control pass first. Official-worker raw
point estimates remain preserved; it does not export raw CIs.

| attacker | main | odds | snap | nearest |
|---|---:|---:|---:|---:|
| Sombra | −152 ± 129 | −167 ± 128 | −169 ± 128 | −131 ± 132 |
| quickstart offline | −596 ± 135 | −628 ± 137 | −647 ± 138 | −647 ± 138 |
| TAG | −466 ± 119 | −464 ± 118 | −466 ± 117 | −466 ± 117 |
| bracket edges | −4,451 ± 579 | −4,559 ± 583 | −5,156 ± 582 | −6,493 ± 582 |
| min bet | −5,983 ± 637 | −6,803 ± 641 | −6,686 ± 631 | −6,686 ± 631 |
| min-raise war | −8,938 ± 736 | −9,373 ± 743 | −9,407 ± 738 | −9,407 ± 738 |
| overbet | −662 ± 197 | −672 ± 196 | −794 ± 197 | −845 ± 199 |
| random size | −6,031 ± 492 | −6,506 ± 488 | −7,554 ± 493 | −8,911 ± 486 |
| tiny shove | −1,802 ± 353 | −1,921 ± 352 | −1,923 ± 352 | −1,923 ± 352 |

Nearest significantly improves over main on bracket-edge and random-size
profiles. Sombra's nearest interval includes zero; do not claim a proven loss
there. These are C-only worker controls, not the independent native-adapter
results or a completed native retention gate. Native SDK/builtin controls and
trained/frozen/LBR/S2/normal-strength checks remain running/pending.

Correct serve/README's stale pre-guard probability and passive-fallback wording
to describe the already retained a51a377/899e09b behavior. Clarify that with
sample:false, mixed-strategy metadata is not the deterministic action law for
AIVAT. This documentation correction changes no API response or selection.

**Native adapter metadata leak found; affected matrix stopped.** Read-only
inspection finds the private native SDK helper used the PolicyFactory seed as
its outward hand ID (and for worker routing). That seed is an invertible mix of
the deal seed: with the public seat/rotation one can reconstruct every hidden
card without knowing the job seed. This is a measurement-adapter defect, not a
new house action or an assertion that Sombra used it. Inspected Sombra uses the
ID as a memory key; its RNG streams use fixed independent constants. Its partial
legacy native baseline is +209.7 ± 103.9, which remains recorded as provisional,
not hidden or claimed to reproduce the real worker's negative score.

Stop only the affected native matrix's process group and idle dependent
coordinators (no other stopped process had children). Preserve adapter-id-hold.json
and all partial scores. Actual arena-worker nine-bot results are unaffected: its
nonpublic-seed IDs already use opaque SHA-based handles. All native trained/frozen
learners and model/source certificates are unaffected. No live service changes.

Replace both outward IDs and process routing with independent 16-byte OS entropy;
ignore the private factory seed entirely. Keep every decision of a hand on its
assigned process; fail closed if entropy cannot be read. New deterministic tests
reconstruct all cards from 6,144 legacy handles, assert new ID/routing generation
consumes only independent bytes, and reject an entropy failure. Fresh helper source
sdk_policy_opaque_v1.rs SHA256
d896eaedebc474eae4b567bfe314b065eed5d3732d8d27599499d9605745dbe6;
source-bound fresh build target native-sdk-opaque-build-v1. The underlying H17
seven-package source remains unchanged and certified by all 205 release passes.

Rerun the entire native C/S2 15-profile/four-arm matrices at their same registered
cards/seeds/sample counts; first exact native positive-reference replays remain
mandatory. OS-random handles/routing deliberately contain no deal-seed dependency;
stateful bot histories remain a distinct statistically evaluated attack. Real
worker comparisons are not pooled with it. Fresh native directories become
native-sdk-controls-{c,s2}-opaque-v1; idle C/S2/full/h2h registries become
native-full-c-clean-v4, native-full-s2-clean-v2, native-h2h-final-v2. Keep all old
registrations, scores and hold receipts; no phase-release or retention marker is
forged for an interrupted matrix. All original C/S2/no-regression gates remain.

**Native identity tests pass; clean feature-only composition prepared.** All
three new opaque-adapter release tests pass, zero failed/ignored: legacy IDs
recover all 6,144 complete deals; new IDs/routing depend only on independent
entropy; entropy failure aborts. The new adapter builds, exactly replays the
native positive reference, and reproduces completed deterministic quickstart/TAG
statistics before fresh Sombra/probe/builtin measurements. Underlying source and
house policy remain unchanged. No confidence claim uses the interrupted matrix.

Prepare the optional frozen-learner extraction feature alone on main ad5b35b,
without any native handoff/odds/menu/cache changes. Archive
462b55b981d238588b9303782b66003de9ada76d45939cc901775e5ab7d702cf
changes only exploit::abr/cli and adds four tests. Build only if the registered
C feature-gain gate passes, otherwise record an explicit skip. After C opaque
SDK and C extraction stages finish, seven nice-10 build workers CPU15–21 plus
original11/additionalC4 stay within22. Fresh target/full seven-package release
suite and exact source hashes are mandatory; final S2 gain/default and selected
main-arm replays still precede any commit. Driver build_learner_fallback_main_v1.py;
receipts hardening-19/learner-fallback-main-build-v1. This prepares a reviewable
composition; neither it nor any native policy code is retained yet.

**Use the existing observational audit while measuring frozen attacks.** Before
any full-coordinator scoring starts, add the already tested --audit-handoffs
flag to its frozen evaluations. It preserves decisions/estimates and reconstructs
public histories to attribute raw payoff and fallback reasons by attacker seat
and first handoff street. These descriptive conditional means have no independent
CIs and are not causal effects; missing-infoset fallbacks are excluded. Exact
old/new/pressure replay assertions remain mandatory, so altered scoring stops.
Only idle C/S2/full/h2h registries are superseded: native-full-c-clean-v5,
native-full-s2-clean-v3, native-h2h-final-v3. No old score is repeated/reselected,
no model/defender/attacker weight changes, and no extra inference job is added.
This supplies street/seat diagnostics for the next remaining leak within the
registered scoring budget, rather than collecting only an aggregate score.

**Recovered final-model pressure matrix completed; broad-menu resource failure.**
Both full S2 hashes match 0f57b2…, and the frozen 1M control exactly reproduces
+680.907618 ± 94.105321. All twelve original frozen pressure-v1 S2 cells complete
under the recovered boot; no C score or training was repeated. Results/commands
remain in hardening-14/pressure-s2-continuation-v1 for full reporting below.

The newly registered 15-size boundary learner logs 4,096 iterations / 170s,
24 it/s. Its five-minute >=100-it/s resource gate stops it. No 3M archive or
usable attacker score exists; preserve resource-rejected.json and aborted marker.
Its export is explicitly resource-rejected, not a passed independent attacker
or a release of the original-host retention gate.

**New performance experiment registered before timing.** The nearest guard now
hands off moderate off-menu bets too, increasing repeated deterministic preflop
400-sample fallback work relative to the earlier snap-only cache experiment.
Keep that earlier 1.17x CI[0.60,2.30] experiment rejected. On this new workload,
compare the exact H17 source/executable with that source plus only the existing
exhaustively tested bench.bp cache; source
f212560d6f4998e3abc2295cd0d8308e09d2241f6a4c2143a7b6ec101e776431.
Fresh target and full seven-package 207-pass/40-suite release gate precede timing.
No policy/menu/equity formula changes; all 6,630 card/opponent keys must be bit-exact.

Fixed five paired seeds 81–85, alternating old/cached order, 512 iterations each,
one worker CPU12, 5-GB store, blind seats, pressure-boundary-v2, verified S2 with
odds/snap/nearest enabled. Measure full subprocess wall time, including table
warmup, model/allocator, training, evaluation and archive writes; retain actual
training seconds as secondary metadata. The 256-deal seed2026110686 evaluation
is solely an exactness witness, not an attacker-strength result. Require every
native archive hash, learner counter/miss rate, EV and target diagnostic to match
bit-for-bit per pair. Repeat seed81 once per arm afterward as a deterministic
archive check, excluded from the five timing pairs. First exact frozen S2 1M
reference is mandatory. Paired log-speedup Student-t 95% lower bound must exceed
1.20 (df4); otherwise no cache or accelerated full run is kept. No sample tuning.

Only the task-owned secondary worker is used; the aborted trainer and all prior
S2 scoring have ended, with no other model job. Drivers build_boundary_cache_v1.py
and boundary_cache_performance_v1.py, receipts hardening-20/boundary-cache-{build,
performance}-v1. Full training, final composition/strength and any source retention
still require separate validation; no poker gain is claimed by this timing study.

**Complete preflop-table cache rejected again on the broad workload.** Full
207 release passes/40 suites and frozen S2 reference pass. Every timed/repeated
learner archive, counter, EV and target diagnostic is exact. Five full-process
512-iteration speed ratios are 2.1683/0.4682/2.3692/0.4972/2.0834; geometric
1.2003x, paired log-t 95% CI [0.4264,3.3789]. The lower bound fails 1.20; retain
no global cache or accelerated 3M run. First runs at each seed take about 57–68s,
second runs 28–29s regardless of source; repeated seed81 is old29.32/cached29.50s.
This demonstrates order sensitivity, not its cause. Preserve the rejected result.

**Per-sampled-deal memo experiment registered before building/timing.** A new
approach caches all streets' deterministic fallback equity inside one CFR deal,
keyed by actual actor, street and remaining-opponent count: 120 OnceLock cells,
about 2KB per private traversal context. Recalculate state-dependent price/action
each time. No global precomputation, no cross-hand cache, no live/API private-card
timing state; benchmark and serving fallback still use the original calculation.
Source c864920543b48079d2f70f8701226867312aac3318b568eb1fca92e630e72577
changes only bench.bp and exploit.abr relative to H17. New tests cover real cards,
all streets/prices/player counts, exact equity bits, repeats and separate deal
lifetimes; retain existing training-versus-evaluation parity tests. Fresh full
seven-package207/40 release gate precedes inference.

Fixed five new paired seeds91–95, 512iterations, one worker CPU12, 5GB, blind seats,
pressure-boundary-v2, same verified S2/three guards. One untimed baseline priming
run per seed precedes both timed arms to evaluate warmed-model computation
explicitly; do not reuse/reselect prior cold timings. Alternate old/cached order,
repeat91onceperarm afterward, excluded from the estimate. All priming/timed/repeat
native archive hashes, learner counters, EV and target diagnostics must match,
as must the frozen S2 control. Measure full process time including model open,
allocator, training, evaluation and writes; retain child CPU/system time, major/
minor faults and I/O blocks to assess order effects. Require paired log-t95%
lower speedup>1.20(df4); otherwise no memo or accelerated run is kept. The
256-deal seed2026110686 sample is an exactness witness, not an attacker bound.
No concurrent computation on this task worker. Drivers build_ctx_cache_v1.py and
ctx_cache_performance_v1.py; fresh hardening-22/ctx-cache-{build,performance}-v1.
No source retention or model-strength claim follows merely from a timing win.

**Guaranteed-chop leak reproduced in unchanged code.** A separate direct engine
witness links explicit SHA-pinned baseline libraries, whose bench.bp matches
current main. Ten legal river nodes (2–6 players, 50/99BB bets, royal board) all
make fallback fold: −100 chips versus zero by calling, an exact conditional
+1,000mbb/node. This is a local payoff witness, not an unconditional model score
or statistical CI. Receipt: hardening-21/locked-board-discovery-v1, command
`nice -n 10 taskset -c 18 witness`, with source/explicit library hashes recorded.

Prepare a separate main-only fallback guard for universally shared river boards:
royal flushes, rainbow Broadway (at most two of a suit), and quads with the highest
remaining kicker. Three new native tests enumerate all2,598,960boards, verify all
709,136 possible holdings on the656recognized boards, reject near-locks, and
check exact chip payoffs plus every legal continuation of the constructed
50/99BB river family, including unequal-stack sidepots. A new authenticated
HTTP/batch test covers30+30decisions at five chip scales and three board classes.
Source3e33488a2789ffb284d0b2296e5f7eb78d33cb398f7201f9e18b5fe68992c38a
changes only bench.bp and the new HTTP test relative to main; no native/menu/cache
prototype is copied. This source is prepared but unbuilt/unretained; full remote
checks, model controls and normal-strength validation remain before any commit.

**Recovery checkpoint, 2026-11-06.** Detached original-host jobs survived the
agent interruption. The first new 3M blind-seat residual attacker completed and
reloaded exactly: +11,655.916 ± 254.174 against the odds prototype on seed0513.
Main at seeds0513/0514 scores +15,931.668 ± 261.750 / +15,969.609 ± 260.899.
These are focused trained-seat estimates, not all-six-seat tournament scores.
The full coordinator must still test snap/nearest against these frozen weights;
no claim that the remaining leak is fixed is justified.

The isolated optional learner-fallback composition passed189 reported tests in40
suites (17 existing fixture early returns), zero failed/ignored. Before retaining
it, replay its default reference and selected adaptive1m/argmax strength/passive
main-arm cells at both held-out seeds0661/0662, 100k deals each. Require exact
exploitation, miss rate and target diagnostics against the certified H19 outputs.
Driver replay_learner_fallback_main_c_v1.py uses3 nice10 workers CPU15–17 while
C cache remains active; total18 workers. S2 gain/composition checks remain pending.

Queue build_locked_river_v1.py on the task-owned secondary worker after the
registered memo timing experiment completely ends, avoiding timing interference.
Use a fresh source-bound target, full seven-package release tests/builds,
expected189 reported tests across41 suites, zero failed/ignored, and mandatory
exhaustive board/payoff/API tests. Twelve nice10 build workers; no other secondary
compute during these builds. This authorizes validation only, not source retention.

**Priority residual diagnosis, registered before new arms.** Since the new
3M blind-seat attacker remains strongly profitable, run its already registered
snap/nearest cells sooner in hardening-23/residual-priority-c-v1. First certify
the full archive with buffered/direct hashes and exactly replay odds seed0513.
Then average/argmax × seeds0513/0514 × snap/nearest,100k focused deals each,
with handoff audits. Four nice10 workers CPU18–21:11training+4fullC+3clean-main
replay+4priority=22 maximum. These duplicate registered full-C cells are diagnosis,
not independent samples; never pool them or skip full S2/strength gates. Driver
residual_priority_c_v1.py emits no cache-release marker or source promotion.

**Completed independent narrower-pressure S2 continuation.** The original
pressure-v1 3M archive (seven residual sizes, outside-blind seats fold) completed
all12 final-model cells after full buffered/direct hash checks and an exact
historical frozen reference replay. This is the recovered old pressure study,
not the resource-aborted15-size boundary study. Each row uses100k duplicate
deals, all-six-seat average,95% corrected CI; no cross-row pooling.

| Frozen extraction / seed | Main | Odds | Snap handoff |
| --- | ---: | ---: | ---: |
| average / 2026110527 | +107.09 ±31.98 | −6.67 ±30.95 | −126.39 ±30.14 |
| average / 2026110528 | +99.79 ±32.51 | −14.47 ±31.42 | −121.88 ±30.54 |
| argmax / 2026110527 | +85.60 ±27.14 | +6.62 ±26.36 | +6.97 ±26.19 |
| argmax / 2026110528 | +84.96 ±27.47 | +3.44 ±26.63 | −6.17 ±26.28 |

The frozen attack profits significantly against main in every row. Snap's argmax
intervals still include zero. This table establishes neither a profitable attack
on snap nor an upper exploitability bound. Exact commands, raw CIs and per-seat
estimates remain in hardening-14/pressure-s2-continuation-v1; all-studies.done
records completion. Native policy promotion still waits for the broader gates.

The priority driver v1 stopped before inference: it looked for the source hash in
the binary receipt, while this clean build records it in input-identity.json.
Preserve that KeyError and use v2 with the same binary hash plus the correct
source-manifest assertion, matching the existing full-C validation. Fresh output
residual-priority-c-v2; no measurements or gates were changed or discarded.

**Conditional stronger-attacker recovery registered before training.** If and
only if H22's fixed five-pair exactness/performance gate passes, use its identical
policy/weight computation for a fresh3M boundary-menu attacker, training seed96,
5GB store,12workers, blinds0/1, against all three experimental guards. Wait for
the separate locked-river build to finish first; that policy change is not used.
Keep the original five-minute100it/s resource floor; abort honestly if it fails.
Focused exact reload uses seed2026110692; held-out average/argmax × four defender
arms use seeds2026110693/0694,100k all-seat duplicate deals, fold outside blinds.
The original seed68 resource rejection remains intact. A new source-bound S2
reference replay must pass first. Driver boundary_s2_study_v2.py, fresh H14
boundary-s2-study-v2. This is attacker training, not a new defender blueprint.
No cache timing, negative attack or partial matrix authorizes policy promotion.

**Per-deal memo performance passed.** Five preregistered warmed training pairs
speed up5.4938/5.3688/5.5649/5.5287/5.4122×; geometric mean5.4732×,
paired log-t95%CI[5.3728,5.5754], above the1.20floor. All five priming/timed
archive hashes, counters, evaluations and target diagnostics match exactly;
both repeated seed91 arms also match, and the historical S2 reference replays.
Full207reported release tests/40suites passed,0failed/ignored,17existing fixture
early returns. This is measured training speed on the specified workload,
not a stronger defender or universal cold-start claim. Keep old H15/H20 failures.

Isolate only the bounded memo onto main774209a in source
eddbb8b16b5883fe0a4332a5ea7c55d4af4499a3cf5148787450a41576216d5f.
Only bench.bp/ABR change; no experimental defender/menu/extraction/locked-board
code. Queue build_ctx_cache_main_v1.py after the locked-river build and the new
3M run's five-minute resource gate, so it cannot distort that gate. Four nice10
workers CPU12–15 plus12training <=16 on the task worker, fresh target, full187
reported release tests expected across40suites. Exact main C/S2 compatibility
and composition measurements remain required before retaining this source.

**Locked-board validation found a test assumption error.** Exhaustive board/rank
and near-lock tests passed, but the continuation enumerator expected a short
all-in's minimum to exceed its maximum. State::legal explicitly clamps that
minimum to the maximum (engine/nlhe/src/lib.rs), so a one-action raise range is
equality. Change only that assertion to equality; no policy logic is changed.
Preserve the failed full suite. Fresh source61daa3a043f6ba7a62a846e0f0d8487bf59324f42ca7eac6ffadd63b42d13467,
locked-river-build-v2 repeats the entire release suite in a fresh target.
The conditional training/cache-build/export queues correctly failed closed on
this prerequisite. Fresh boundary-s2-study-v3, ctx-cache-main-build-v2 and export
v3 preserve the same source, training seed96, held-out seeds and gates; no poker
measurements ran in v2 and no failed marker is overwritten.

**Priority residual average-policy results, two fixed seeds.** The new 3M
blind-seat attacker archive passes full buffered/direct verification and exact
odds replay in H23 v2. Focused blind-seat estimates,100k duplicate deals each,
corrected95%CI (not full-table scores):

| Seed | Main | Odds | Snap | Nearest |
| --- | ---: | ---: | ---: | ---: |
| 2026110513 | +15,931.67 ±261.75 | +11,655.92 ±254.17 | −14,055.48 ±152.80 | −14,068.71 ±152.84 |
| 2026110514 | +15,969.61 ±260.90 | +11,687.17 ±252.93 | −14,163.04 ±152.25 | −14,181.10 ±152.38 |

Raw nearest estimates are−13,914.54 ±280.51 /−14,074.37 ±282.61; the improvement
is not specific to AIVAT. Snap and nearest do not separate significantly from
each other here. Argmax cells, broader native controls, independently adapted
final-model attack and normal-strength gates remain pending. These are frozen
weights trained against odds: no claim they are a best response to either handoff.

All five optional-attacker clean-main C replays passed exactly: default+419.452
±95.368; adaptive1m/argmax strength−451.417 ±37.659 /−496.320 ±37.693;
same-archive passive−1,187.257 ±45.638 /−1,223.633 ±45.494. Match full EV,
miss rates, diagnostics and mode labels to H19's measured main arms. This verifies
composition, not a new independent strength sample. S2 remains required.

**Clean-main memo retention gate registered.** H22's5.47× gain used experimental
handoff guards; it does not establish a gain on current main. After the secondary
boundary study finishes (or its declared resource abort), run isolated main memo
versus the certified baseline with all prototype guards disabled: adversarial
menu,blindseats0/1,4,096iterations,5GB,one worker, seeds101–105. Prime once per
seed, alternate both timed arms, repeat101once per arm outside timing. Require
exact full archives/counters/EV/diagnostics, exact historical S2 reference, and
paired log-t95%lower speedup>1.20. Evaluation256deals seed0696 is only an
exactness witness. No concurrent secondary compute; no main cache retention
without this test and C compatibility. Driver ctx_cache_main_performance_v1.py,
fresh H22 ctx-cache-main-performance-v1. This avoids retaining a claimed speedup
that only exists under experimental defender options absent from main.

**Broader S2 recovery failed its unchanged resource gate.** After the exact
S2 reference replay, seed96's memoized run reached only21 iterations/second at
the five-minute check. It stopped without an accepted learner archive or score.
Sampled workers were blocked in folio_wait_bit_common; process reads exceeded
5.99GB while CPU utilization was low. Thus the demonstrated warmed computation
gain did not solve this run's model-read bottleneck. Preserve the resource receipt
and reject this attempt; do not relax the gate or import/export nonexistent results.

Next resource hypothesis: use the original larger host with its active verified
C model. Compile the unchanged source c8649205… in fresh H24 ctx-cache-c-build-v1,
full207-report/40-suite release checks,7 nice10 workers CPU15–21 after H23 ends;
original11+fullC4+build7 <=22. No S2 model load on this host during its C phase.
A subsequent candidate-adaptive C training run must hold the existing original
C scheduler while active and release it in a guarded finally block, preventing
a cache-phase transition during this additional work. No live service is touched.
The failed final-model adaptive requirement remains outstanding, not satisfied
by a future C-only result. Main-only cache build/performance proceeds independently
on the task-owned secondary worker after its aborted S2 stage.

**Candidate-adaptive C recovery registration.** After H24's clean build, train
the fifteen-size boundary attacker for3M iterations on C against all three
guards, fresh seed97,5GB,7workers CPU15–21. Exactly replay the certified old1M
C reference first. Keep the five-minute100it/s floor; exact focused reload seed
2026110702; average/argmax × main/odds/snap/nearest held-outs use0703/0704,
100k all-seat duplicate deals with outside-blind folds. Driver
hardening-24/boundary_c_study_v1.py; no automatic retention or phase release.

Its supervisor checks boot, start time and exact command line, then holds only
our original H12 scheduler while leaving its current training child running.
This prevents C-to-S2 cache transition during the added C work. A finally block
terminates any still-running owned child and resumes that same scheduler on
success, failure or handled termination. CPU cap remains11+4+7=22. The current
UTG run is preserved; no live process/service is signalled. Any new defender
regression must be included in final retention review, even if older matrices
pass. This cannot replace the still-missing independently adapted S2 result.

**Guarded final-model recovery queued on the larger host.** If H24 C adaptation
completes, boundary_s2_original_v1.py waits for H12 to finish every C job and pass
its final-model frozen reference, then holds only that same scheduler before its
S2 measurements-complete marker. Its current child may finish, but downstream
SDK/full-model queues cannot start while the scheduler is held. This allows7
workers CPU15–21 alongside at most11 original workers, S2 only. Boot/start-time/
command checks and the same finally-resume/child-cleanup guard apply.

Fresh seed98 trains3M iterations,5GB,all three native guards, fifteen-size menu;
the five-minute100it/s floor is unchanged. Exact focused reload0712; held-out
average/argmax × four defender variants use0713/0714,100k all-seat duplicate
deals with outside-blind folds. If the C prerequisite resource attempt fails,
record an explicit skip. If the S2 insertion point was missed, stop instead of
risking overlap. This is the registered recovery for the missing adaptive final-
model evidence; old failed secondary attempts and their import markers stay failed.

**Final gate wiring updated for the resource recovery.** The old independent
import waited for the failed secondary seed68 archive, which must never be
fabricated. Preserve it as superseded, stop only that idle owned importer and the
idle v3 final-h2h coordinator, and use review_adaptive_original_v1.py plus h2h v4.
The new reviewer requires both H24 C/S2 studies complete, both schedulers resumed,
source/binary/model/archive identities, resource floors, exact references/reloads,
and all32 held-out rows. It recomputes every raw/corrected disjoint-CI gate and
rejects candidates with any newly found regression. Resource failure fails closed.

Normal-strength v4 changes only that prerequisite: all prior full C/S2 controls,
the older odds-only h2h completion-or-explicit-rejection check, main historical
+5.09 ±1.16 overlap,200-deal smokes and4M all-format same-seed measurements remain
mandatory. No candidate or incomplete adaptive run can bypass these gates.
No code promotion or deployment is automated by this wiring.

**Completed current validations.** H21 locked-river v2 passed189 reported
release tests across41 suites,0failed/ignored,17existing fixture early returns;
all exhaustive card/continuation and authenticated HTTP/batch checks executed.
H22 clean-main memo v2 passed187 reported tests across40 suites,0failed/ignored,
17fixture early returns, and completed its separate release build. Its registered
main-configuration S2 timing/reference experiment has started; no source is kept
on the basis of the earlier experimental-policy timing result alone.

H23 priority residual diagnosis completed all eight snap/nearest cells with no
raw/corrected disjoint regression within that matrix. Argmax nearest at the registered seeds2026110513/0514, giving−14,015.02 ±149.36
and−14,161.77 ±148.86. These complete the focused C frozen-policy evidence only;
the newly queued adapted C/final-model studies and normal strength remain required.

### Public arena privacy shutdown (2026-11-05)

The user requested that the displayed bot/reference standings no longer be public.
The public edge now fails closed unless PUBLIC_ARENA_ENABLED is exactly "true";
the deployed configuration sets "false". All public API paths and methods,
including result lookups, new submissions, inference and WebSocket handshakes,
return a generic 503 before any origin fetch. Responses are no-store/noindex.
Worker development/preview URLs are disabled. The play and standings pages are
replaced with static unavailable notices without data-fetching scripts, and
operational run summaries are removed from public site content/model-card drafts.

Records are retained for private validation. No origin service, model, image,
database record or system configuration is stopped, rebuilt, deleted or changed.
The GitHub repository is private; detailed evaluation receipts remain there and in
the existing private work directory. Reopening public access requires an explicit
decision after validating the intended code/model deployment, plus restoring the
public UI; switching the edge flag alone is not a validation procedure.

Five remote Node tests passed (zero failures), covering default/malformed flags,
query/auth/header/encoded-path bypass attempts, writes, WebSocket upgrades, HEAD
responses and the preserved forwarding/header behavior when explicitly enabled.
The tests mock origin fetch and assert zero origin calls during maintenance.
Node v22.23.2 was downloaded from nodejs.org into the private work directory and
verified against its official SHA256 manifest; no system installation was made.
Command: nice -n 10 node --experimental-strip-types --test --test-concurrency=1
arena-proxy-worker.test.ts, under ~/fix/public-arena-private-20261105.
All 15 HTML pages pass relative-link/JSON-LD checks, notice pages have no scripts,
operational row identifiers/scores are absent, and skill/model-card mirrors match.

**Live verification passed:** public leaderboard/track/run/player/match reads,
job POST, HEAD and WebSocket handshake requests return 503 without result rows.
Both www/non-www play and standings pages serve only unavailable notices with
no-store/noindex headers. Unauthenticated direct-origin access returns 403.
The internal origin remains ready and retains all 16 current leaderboard records.
Edge version: 9d8814a7-0594-47dc-8497-e73494cd01ff; static site version:
bfda52bd-4c9d-48e3-97bf-d1a1298036a9. No public reopening was performed.

### Original sombra submission rerun (2026-11-05)

The user requested another run of the original submission against the current
hosted path and the latest retained fixed policy. Retrieve the artifact belonging
to original job job_f5e9a193262bed4c / deleted historical run w_4bb6e197e24a
through the runner's artifact API. Its exact compressed SHA256 is
35414556b62e652763e5355e368d2b25da0d2b1ca9d755b051c54ae31a290d00.
All 14 Python/data files match the previously tested local sombra copy. No entrant
contact/profile data is printed or retained. Source is not committed here.

**Fresh hosted result:** **+4,760.7 ± 491.7 mbb/game for sombra**, 20,004 hands,
fresh duplicate deals + AIVAT, verified, zero forfeits and successful sandbox
teardown. Job job_e3dc8893ea19c662, new operator player p_e1d1e121fba3,
[run w_da35215b0163](https://arena.sharingan.poker/v1/benchmarks/sharingan-1/runs/w_da35215b0163).
This uses the existing hosted worker, C@2e10 track sharingan-1, two lanes and the
unchanged original archive. It does not recreate the deleted historical row.
The current hosted path remains significantly profitable for this attack.
No serving binary, image, model or service configuration was changed.

Driver sharingan-sombra-live-rerun.py and sanitized artifact identity, commands,
receipts and gate live-rerun.ok are under ~/fix/sombra-rerun-20261105.
The trusted operator submits POST /v1/jobs in worker mode with hands 20004,
lanes 2, entry bot.py, artifactTgzBase64 containing the exact original gzip,
and a stable idempotency key. Bot code runs unprivileged in the existing isolated,
network-disabled hosted sandbox; credentials stay outside the bot process.

**Fixed-policy reruns registered before their results:** after the live job ends,
run only on the existing work host as ubuntu at nice 10, with no credentials in
the child environment and no-new-privs inherited by bot processes. Use immutable
arena-odds SHA256 43a141acf05e71fd8dc64c966b9fc85700bd63022682c690a52b1d2b0fbdb6f7,
with ARENA_PASSIVE_ODDS=0, the validated fixed baseline; the unfinished odds
candidate is disabled. This baseline has previously reproduced the retained
policy exactly. No new Rust policy change is made.

Verify the full private C file against published SHA256 2a4484f6… before any C
inference, then run the old-leak positive control at seed ab-20004 with
ARENA_TRACK=pre-sync-fix. Require its lower 95% bound above +1,000. The fixed
reference at the same seed must exactly match all prior score/CI/hand/error fields
in hardening-08/sombra-base.json before interpreting a fresh fixed sample at
seed sombra-rerun-20261105-v1. Then switch to final SHARINGAN-2, verify its full
SHA256 0f57b296…, exactly replay hardening-08/s2-sombra-base.json at ab-20004,
and evaluate the same fresh seed. Each run uses 20,004 hands, eight lanes,
medium sizing and buckets-200. There is one model job at a time; live and private
phases do not overlap. Stop and investigate any hash/reference/control failure.

Exact private command form, under taskset CPUs 0–15 and setpriv --no-new-privs:

~~~sh
~/fix/venv/bin/sharingan eval ~/fix/sombra-rerun-20261105/submission/bot.py \
  --local --track sharingan-1 --hands 20004 --lanes 8 --seed SEED \
  --quiet --out RESULT.json
~~~

The driver records the complete constructed environment and absolute argv,
source hashes, model hashes/stat/boot checks, reference gates and output files.
Fresh live and private samples use different seeds; do not treat them as a paired
causal ablation or pool them. Until these checks finish, the new live profit and
the prior fixed offline losses remain separate findings; neither a source-version
diagnosis nor a new poker-policy improvement is established.

**C checks completed:** the full private file hash matched published SHA256
2a4484f6fabf61470a67c941552f5a97c6f6474e14ea010ef94a41d7b5861300
with unchanged metadata. The old-leak positive control exactly reproduces
**+5,381.8 ± 513.9**; the fixed ab-20004 reference exactly reproduces
**−419.6 ± 161.6**, including all recorded hand/error fields. On the registered
fresh seed the original submission scores **−467.1 ± 161.8** against the fixed
C@2e10 policy, again 20,004 hands and zero forfeits, crashes or timeouts.
The live +4,760.7 ± 491.7 and fresh fixed loss have disjoint intervals, but use
different deal samples and deployment/input paths; this establishes a behavior
gap, not a controlled diagnosis of its cause.

**Final SHARINGAN-2 checks completed:** the full private S2 file hash matched
published SHA256 0f57b29605cb0fc5f2ecf4c1fcb266df6321d8459784b1577a30d54bb94c5b8f
with unchanged metadata. The fixed ab-20004 reference exactly reproduces
**−390.4 ± 155.5**, including all recorded hand/error fields. On the registered
fresh seed sombra scores **−526.5 ± 154.4** against final S2, 20,004 hands and zero
forfeits, crashes or timeouts. The original 14 source/data files still match their
initial hashes after all evaluations. Marker fixed-rerun.ok is present.

| fresh rerun target/path | sombra mbb/game ± 95% CI | hands | forfeits |
|---|---:|---:|---:|
| current hosted C@2e10 worker | **+4,760.7 ± 491.7** | 20,004 | 0 |
| validated fixed C@2e10 policy | **−467.1 ± 161.8** | 20,004 | 0 |
| validated fixed final SHARINGAN-2 policy | **−526.5 ± 154.4** | 20,004 | 0 |

The hosted row is a real signed/verified job. Private rows are controlled offline
evaluations, not manually inserted verified leaderboard scores. No weights or Rust
policy were changed by this rerun, and no serving deployment was performed.
The private tests confirm the retained policy's response to this specific attack;
the stronger profitable frozen learner and separate API weaknesses remain.

**2026-11-05 live leaderboard E2E:** at the user's request the old sombra entry
and its four runs were deleted through the supported admin API. Four fresh operator
bot entries completed real hosted, signed, verified 20,004-hand runs with zero
forfeits and sandbox teardown. An unranked executable-identity diagnostic could not establish the deployed
source version: its marker check failed a positive control, so the initial
version-mismatch interpretation was withdrawn. These live results do not establish
deployment of the fix or a current live sombra loss. No services/images/models were rebuilt or reconfigured.
See [the E2E results and runtime evidence](LEADERBOARD_E2E.md).

**2026-11-04 model-integrity audit:** numerical C evaluations using the original
`/srv` file are paused. Repeated
read-only reads of the provided C file returned different bytes at the same offset,
with unchanged inode, size, mtime and ctime. The public copy and one direct read agree;
other reads return zero pages in that key-index region. Historical C estimates below
remain qualified unless reproduced on verified inputs. The frozen 1M attack reproduced
exactly on the independent public C copy (+419.5 ± 95.4). Both original frozen 3M seeds
also reproduce exactly on a freshly downloaded private public C copy, confirming their
pooled +2,306.7 ± 75.2. The seed-5 LBR result also replays exactly on that copy
(strong +841.4 ± 161.7, weak +330.0 ± 164.8). This does not clear the original
read path or validate every historical run. No original model, RAID configuration or
live service has been modified. See iteration 7 below. This is an input-integrity concern, separate
from the fixed translation bug.

**Strongest confirmed trained attack against final SHARINGAN-2:** the 3M / 35-GB policy wins
**+2,465.9 ± 74.9 mbb/game against final SHARINGAN-2** on verified inputs,
two deal seeds / 1.2M hands, duplicate + AIVAT, without search. Its confirmed C@2e10
result is +2,306.7 ± 75.2. The mixed-passive-odds candidate remains uncommitted and is
not part of these baselines. These are profitable deviations, not equilibrium-gap
estimates or a comparison of normal playing strength.

Separately, iteration 10's new near-all-in attacker profits against verified C:
the actual API scores +14,283 ± 910 at 500/1,000 blinds, while the native canonical
50/100 game scores +3,965 ± 788 on discovery deals. These are different target
paths and correction schemes; no final-S2 near-all-in result is available.
The measured mixed-odds candidate is not retained on main because its remaining
final-model and normal-strength gates were unfinished when the user stopped the loop.

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

**Verified final-model follow-up (iteration 7): the frozen 1M attacker wins
+714 ± 67 against SHARINGAN-2 over two independent-deal evaluations (1.2M hands),
using fully verified model and learner files.** The same policy exactly reproduces
+419 ± 95 against an independent verified public C copy. Stronger original-host C
figures remain qualified by the input audit above.

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

### Hardening iteration 7 — shadow parity, input integrity and verified S2 attack (2026-11-04)

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
Baseline SHARINGAN-2 evaluations of the same frozen policy then completed, seeds
271829 and 314160, again 100k deals per seat each. No candidate defending policy is used.

| SHARINGAN-2 evaluation seed | AIVAT mbb/game | raw mbb/game |
|---|---|---|
| 271829 | **+680.9 ± 94.1** | +680.4 ± 112.7 |
| 314160 | **+747.5 ± 94.2** | +705.8 ± 112.4 |
| pooled, same frozen policy | **+714.2 ± 66.6** | +693.1 ± 79.6 |

This is a confirmed profitable deviation against five copies of the fixed **final
SHARINGAN-2 blueprint without search**, using an attacker trained against C. The target
and frozen learner passed their independent full-file hashes. It is not an equilibrium-gap
estimate and says nothing by itself about search strength or normal head-to-head ranking.
There are 200,000 independent duplicate-deal clusters, six attacker seats per cluster,
**1,200,000 hands**. Pool the two fixed-policy samples using their within-sample sums
of squares plus the between-seed mean term; never treat the six duplicate hands as six
independent observations. Pooled per-seat mbb/game (seat labels 0 through 5) are
+601 ± 145, +1,432 ± 158, +942 ± 120, +581 ± 111, +550 ± 108, +178 ± 109.
Those seat-level intervals are descriptive, without multiple-comparison adjustment.
Artifacts `public-s2-frozen1m-discovery.json`, `public-s2-frozen1m-holdout.json`,
`pooled-s2-frozen1m.json`; evaluation runtimes 351.6 and 167.0 seconds, excluding archive
loading. The stronger saved 3M attack has since been confirmed on final SHARINGAN-2,
as recorded below; its C reference seeds also reproduce on verified input. No playing-policy A/B or
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

**Private reconstruction rejected:** the copied/repaired incoming file failed its first
full buffered SHA-256 check: `96ba366924704018287ae68c2f5466cccfded7c17b4f56ca5a89d4e7d5e106a7`,
not the release digest. The source-copy digest was `fd132496...`, a further observed
read variant despite unchanged source metadata. Do not infer that the known 20-KiB
region accounts for all inconsistencies. The incoming file stays as evidence, is not
published under the final model name, and is never used for evaluation. The preparation
parent exited before starting 3M compression. Both conditional experiment queues then
exited because their input marker was absent; **zero iteration-8 poker measurements
were run**. This is a failed input reconstruction, not a measured policy regression.

Switch to `hardening-07/sharingan-download-c-v1.py`: fresh public C parts pinned to the
same revision/LFS hashes as the independently verified worker. Assemble only verified
parts, fsync/checkpoint each append, then require full buffered and direct SHA-256 to
equal `2a4484f6...` before publishing the fresh private file. It copies no bytes from
the original model. Keep the failed reconstruction and all logs; no live/model/storage
configuration changes are authorized. Start the independent 3M compression/checksum
step separately only after process/output checks prove the old parent is terminal and
that this step had never started (PID 886597, `prepare-3m-independent-v1.log`). Its native
buffered/direct equality guard is unchanged. Subsequent experiment drivers must depend
on the new download proof and the independent archive job, not the failed reconstruction.
Fresh-download PID **886877**, log `public-c-download-v1.log`. The failed-copy file is
preserved under its `.incoming` name; all four old copy-dependent processes are confirmed
terminal. Frozen v3 (PID **889457**) replaces only those failed input dependencies,
reading `private-c-download.json` and the independent 3M producer's marker. Its public
region stability guard, binary/source/learner checks, reference replays, seeds, sample
counts and statistical comparisons remain unchanged. No policy-result data informed
this input change. Keep original v1/v2 logs and scripts for the failure record.

Prepare the independently checked 3M attacker for a later SHARINGAN-2 baseline confirmation
while C inputs are restored. A task-owned Mac watcher (`sharingan-relay-3m-v1.py`, PID
65621) waits on the live source producer and its native buffered/direct/compressed
verification marker. It then relays only the compressed learner through a bounded
Mac temporary directory, checking available space and byte length. No full C or SHARINGAN-2 blueprint file is
copied to the Mac and no build/training job runs there. Full compressed/native
SHA-256 checks on the destination remain required; a completed relay is not yet an
accepted match input. The independent compute worker is asleep until this data is ready.
Twelve small probes of the failed incoming copy (eight native direct reads with
4-KiB/64-KiB blocks plus four buffered reads) all match the public digest for the patched
64-KiB region. `failed-copy-region-reads.json` records them. The known-region repair
therefore did not make the full copy valid; no further guessed patches are attempted.

The current broader queue is immutable `sharingan-odds-broad-v2.py` (PID **891541**),
waiting on frozen v3 and the fresh-download proof. Its earlier v1 process is terminal
with no evaluations started. Only the input dependency and proof path changed; all
external/LBR/new-adaptive commands, gates and sample sizes stay as declared above.

**Fresh public C accepted; frozen references reproduced:** the download completed in
4,564.3 seconds. All five pinned LFS parts passed their individual hashes, and the
assembled 124,681,168,783-byte file passed buffered **and O_DIRECT** full-file SHA-256
2a4484f6fabf61470a67c941552f5a97c6f6474e14ea010ef94a41d7b5861300.
The only C input for new experiments is now
/home/ubuntu/fix/models/sharingan-1-verified.plcp; the rejected incoming file remains
preserved. All 36 additional region reads matched 62b40add…, with unchanged metadata.
Native learner buffered/direct hashes also agree: frozen 1M 169ef576…, frozen 3M
1706c51f4ce63cc8052d1886765fed69bfec9461c1e03ac97207e82a4e0cf951,
and iteration-6 adaptive 1M 7ca03101…. The iteration-8 disabled binary exactly
reproduced the complete original 1M reference (271829) and **both** original 3M references
(271828, 314159), including raw/per-seat/pooled estimates, CIs, counts, miss rate and
normalized tracking options. All three replay gate JSONs are true. Thus the frozen
3M bound **+2,306.7 ± 75.2** is confirmed on canonical input; other unrepeated historical
experiments are not thereby validated. LBR is being rerun separately.

**3M transfer and resumed worker:** independent preparation passed native buffered/direct
agreement, zstd integrity and compressed SHA-256. Native size 29,581,123,222 bytes,
compressed size 14,006,459,630 bytes, compressed digest
bb87d61c316d75a29efc278cd92f65c596b1b51e51b7463450fcf30dd9ee4b20;
source metadata digest 618c579487924e801cd02d3ba028bed8f5d07f424032f1f7bffdf0086643292d.
The first Mac relay inherited an extra nice increment (20); it was stopped while its
partial bytes were preserved, then resumed at effective nice 10. That resumed rsync
terminated with **ENOSPC, exit 10**, because its temporary copy needed more free local
space. This is a transfer failure, not an SSH timeout or poker result.

Replace it with task-owned sharingan-bounded-relay-3m-v3.py (Mac PID 19316), retaining
the original prefix. The destination checks the 5,201,821,216-byte prefix against the
source hash, appends/fsyncs/checkpoints it, then acknowledges it before its own Mac copy
is removed. Continue with a two-slot pipeline of **512-MiB** chunks: source hash, local
temporary byte count, destination hash/append/fsync/checkpoint, then removal of only
the acknowledged task-owned chunk. Each destination job has a PID and immutable log;
an observation timeout does not trigger a duplicate append. Full compressed SHA,
native decoded SHA and native readback must still pass before use. No full blueprint
is placed on the Mac; credentials stay there and are never forwarded.

The task-owned extra worker was subsequently found running under a new boot ID
3d1e07ff-d2e7-415f-ac55-7c7013d92277; the cause of resumption is unknown.
Do not reuse an old boot's certificate. Its complete SHARINGAN-2 rehash passed
0f57b296… in 849.3 seconds, then the frozen 1M seed-271829 evaluation with six
workers exactly reproduced the prior public-S2 result, miss rate and tracking options.
No work on the primary worker or any live arena service was changed.

**Stronger final-model baseline preregistration:** immutable
s2-frozen3m-v3.py (SHA bb342375f3b14238741b9ccbc937a349c691021fcbcbd4e079047a4b2818ffc1,
PID 202409) waits for the completed compressed transfer on the extra worker. It then
requires full compressed/native/readback hashes and source metadata identity, relocating
only the bucket path. Reuse only the current-boot full model proof and exact 1M replay
certificate with unchanged file metadata. Run frozen 3M against final SHARINGAN-2
using immutable main binary f3b5eaa4…, six workers on CPUs 4–9, effective nice 10:
100,000 duplicate deals per seat at **271828**, then **314159**, training seed 2,
--iters 0 --mem-gb 35 --learner-sizing adversarial --load-learner attackers/learner-3m.bin.
Target is models/sharingan-2.plcp, buckets data/buckets-200, medium sizing, all six
seats. Exact argv/environment and identities are saved per output. Pool only these
two samples using deal-cluster variance, including between-sample mean differences.
This is a stronger baseline measurement, independent of the uncommitted correction;
no new S2 3M result has been observed at registration.

**Stronger final-model baseline confirmed:** both preregistered frozen 3M evaluations
completed on independently verified SHARINGAN-2, with unchanged native weights.

| deal seed | duplicate + AIVAT mbb/game | raw duplicate mbb/game |
|---|---:|---:|
| 271828 | +2,481.14 ± 105.95 | +2,495.55 ± 124.48 |
| 314159 | +2,450.60 ± 106.02 | +2,478.06 ± 124.35 |
| pooled frozen policy | **+2,465.87 ± 74.94** | **+2,486.81 ± 87.98** |

There are 200,000 independent duplicate-deal clusters (100,000 per seed), six attacker
seats and **1,200,000 hands**. Pool within-sample sums of squares plus the between-sample
mean term, preserving deal-cluster variance. Per-seat pooled AIVAT mbb/game, seats 0–5:
+3,086 ± 152, +3,636 ± 160, +2,566 ± 132, +2,078 ± 125, +1,841 ± 119,
+1,588 ± 121 (descriptive intervals, no multiple-comparison adjustment).
This is the strongest confirmed frozen attack tried so far against the final blueprint,
trained against C, without search. It does not measure an equilibrium gap, search
strength or normal head-to-head ranking, and does not promote a defending-policy fix.

Model full SHA is 0f57b296…; native learner is 1706c51f…, compressed bb87d61c…,
immutable main executable f3b5eaa4…. The relocated manifest SHA is
37b4294706ba987d76d55123eb3b59621980f2f96611ae2b5686a894fb663774.
Only its bucket path changed from the verified source manifest. Commands retain six
workers, CPUs 4–9, effective nice 10, --iters 0 --seed 2, the specified eval seeds,
medium sizing and adversarial frozen learner, 100,000 deals/seat.
Artifacts public-s2-frozen3m-{discovery,holdout}.json, pooled-s2-frozen3m.json,
s2-frozen3m-inputs.json; evaluation times 500.1 and 331.8 seconds, excluding archive
loading. All data stayed on the task-owned compute worker. The separate final-model
candidate queue now starts its disabled replays.

### Hardening iteration 8 — preserve pure decisions while repricing mixed passive odds (2026-11-04, preregistered)

**Hypothesis.** Iteration 6 transferred mass from pure fold into call and from pure call
into fold whenever prices differed. That may explain its adaptive regression, though
this mechanism is not established. Test a narrower, parameter-free correction only
when both abstract fold and call masses are positive. For `q = call/(pot+call)`, multiply
the call/fold odds by `q_shadow/q_real`, renormalize within the original fold+call mass,
and leave every raise entry unchanged. In symbols, with original passive mass `m=f+c`
and `r=q_shadow/q_real`, use `c_new=m*(c*r)/(f+c*r)`, `f_new=m-c_new`.
Pure fold, pure call, zero passive mass, free checks and equal-price decisions must
remain bit-identical. This remains a heuristic for a multiway game, not a range model.

**Candidate scope.** Reuse only the opt-in plumbing/test structure from the retired
prototype, replace its correction entirely, and give it distinct selectors:
`TrackOpts::passive_odds` (false by default), `--passive-odds` / `--no-passive-odds`,
offline `ARENA_PASSIVE_ODDS=0|1`, per-side head-to-head selectors. Match bench, training
target and search. Main/default play stays unchanged during the experiment; no live
configuration is changed. Add pure-support regression tests and retain on-menu,
training/evaluation and bench/search parity fixtures. Run the required release suite
and all six builds remotely, nice 10, at most eight build workers alongside the one
input-preparation process. Do not evaluate against the inconsistent original C file.

**Predeclared gates before any new correction result:**

1. Require the private C reconstruction to pass buffered/direct full release hashes.
   New disabled binaries must replay the frozen 1M and 3M references exactly on their
   original reference seeds (all estimates, CIs, counts and miss rate; normalize only
   the added disabled option in the tracking description). Stop on a replay failure.
2. Primary target is the frozen **3M** attack. Discovery seed **271831**, 100,000 duplicate
   deals/seat/arm, all six seats; same archived weights in both arms. Also evaluate the
   frozen 1M C attack and iteration-6 adaptive 1M attacker at that seed. Require the
   primary candidate upper 95% bound below baseline lower bound, with no significant
   regression for either other attack. A regression is candidate lower bound above
   baseline upper bound, even if both estimates lose. Use the same comparisons on
   reserved confirmation seed **314161** and the same fixed sample size. Failed or
   inconclusive primary improvement means revert; do not tune sample counts afterward.
3. Only after those gates: sombra, all six probes, `ctl_quickstart`, `ctl_tag` at 20,004
   hands per arm with identical deals; strict on-menu probability/trajectory checks;
   paired 20k-deal weak/strong LBR using the verified private C in both arms. Rerun the
   old-leak positive control on the verified input and stop if it no longer finds it.
4. Train a fresh attacker against this candidate: 1M iterations, 15 GB, adversarial menu,
   every seat, seed **8**, 11 workers. Save exact native weights. Evaluate seed **161804**,
   100k deals/seat, require exact candidate reload, then evaluate the same frozen policy
   against baseline; repeat both arms on independent seed **271832**, 100k deals/seat.
   Any significant defender regression rejects the candidate. This finite learner still
   has the separately documented store-full fallback mismatch; a negative score alone
   does not establish robustness.
5. Confirm retained target effects on SHARINGAN-2 with the frozen 1M and 3M attacks and
   the new attacker; repeat relevant external controls. Before promotion, reproduce
   the documented SHARINGAN-2 versus C normal-strength head-to-head (4M deals, 3+3,
   baseline both disabled versus candidate S2 enabled / C disabled), same seeds and
   verified models. Pure on-menu trajectories should be identical; investigate any
   discrepancy. Keep only a significant primary improvement with no significant
   regression elsewhere. Otherwise record the honest result and revert.

All intervals are 95%; a poker estimate is positive for the attacker. Commands and
immutable binary identities will be recorded as each gate is launched. The correction
is unproven and uncommitted; neither code reuse nor the old correction's gains count
as evidence for this formula.

**Implementation and validation started:** eight-file candidate source archive SHA-256
`64939b96766d513caf7765f678f8870163dc3cfc3d8a87f528d31674ec3266e6`, tracked patch
`e9881081d10e9cfba1b7c2a22af5cd305a8dc282e474cf55f45e8cc2f7624d37`, based on
`77f01c4`. Archived under `hardening-08/` with per-file hashes. Source replaces the old
linear transfer with the formula above and includes exact bit-preservation tests for
pure fold, pure call, zero passive mass, and pure passive plus raise mixtures.
`sharingan-passive-odds-tests-v1.sh` (PID 864108) runs the six-package release suite
(required five plus pluribus), then all six builds, nice 10, eight build/Rayon workers
on CPUs 0–7. The separate input-copy process uses one additional nice-10 worker; total
compute stays below the original host's 22-thread allowance. Native-model control test
uses `ODDS_TEST_MODEL`, `ODDS_TEST_BUCKETS`, `ODDS_TEST_OUT` and remains explicitly ignored
in the model-free suite until the verified private C is ready. No new correction score
has been measured yet.

**Frozen queue prepared:** `hardening-08/sharingan-odds-frozen-v1.py` waits for the
validation marker, private-C buffered/direct verification, and the 3M native archive's
buffered/direct hash agreement. Before evaluation it also verifies source/binary hashes,
rehashes the original 1M and old adaptive 1M archives through buffered and direct I/O,
and records those identities. It replays both original 3M seeds (271828 and 314159),
not just the first, plus the 1M reference seed 271829. Reference JSON files are copied
unchanged from the previous records. Exact replay gates cover all pooled/per-seat/raw
estimates, CIs, counts, miss rate and the tracking description with only the disabled
`passive_odds` field removed. Discovery/holdout use the three frozen policies in turn,
baseline then candidate, one inference target at a time. Each command uses 11 workers
on CPUs 0–10, inheriting nice 10 from the driver; command/environment/model/learner/binary
identities are saved separately. Significant raw-winnings regression also stops the
queue. It stops after the two frozen stages; the broader, new-adaptive and final-model
gates remain mandatory before retaining any code. No change to the declared sample sizes.

**Release validation passed:** **156 tests / 32 suites**, zero failed, one explicitly
ignored large-model fixture; all six requested builds passed. The ignored fixture is
not claimed as passed and remains queued for the verified C input. Exact commands,
from `~/fix/sharingan`, with `CARGO_BUILD_JOBS=8 RAYON_NUM_THREADS=8`:

```sh
nice -n 10 taskset -c 0-7 cargo test --release -j 8 \
  -p nlhe -p bench -p search -p sharingan-arena -p exploit -p pluribus \
  -- --test-threads=8
nice -n 10 taskset -c 0-7 cargo build --release -j 8 \
  -p sharingan-arena -p bench -p exploit -p search -p sharingan-serve -p pluribus
```

Immutable `hardening-08/` executable SHA-256 values:

* `exploit-odds`: `b9d8e27aa0489060b3b261733f88fd4a4670e21980574da8a795a1db8e4d7437`.
* `arena-odds`: `43a141acf05e71fd8dc64c966b9fc85700bd63022682c690a52b1d2b0fbdb6f7`.
* `pluribus-odds`: `4df0fc7c0b8e81a11dff0554ceac7cc72a95f9c14dcf8a5f278fd819245b26db`.

Before any evaluation, replace the still-waiting v1 queue (PID 873373) with immutable
`sharingan-odds-frozen-v2.py` (PID **877423**). Authoritative process/log/output checks
confirmed v1 was only waiting for the private model and had started no evaluation;
its termination and reason are recorded in `frozen-v1-retired.json`. V2 adds a necessary
input-stability guard: after both full-file hashes pass, read the previously suspect
64-KiB logical region 24 times through native direct `dd` (4-KiB and 64-KiB blocks),
and 12 times buffered. Every digest must equal the verified public block digest
`62b40adda8eadade3d02d51d05f6acde89f8eb768bf92f80f081b3b6f2fcf0d1`,
with unchanged file metadata. Record every read in `private-c-read-stability.json` and
stop on any mismatch. The metric, seeds, sample counts and statistical gates are
unchanged, and no new correction result has been observed.

**Conditional broader queue:** `sharingan-odds-broad-v1.py` runs only after
`frozen-gates.ok`; it exits if the frozen driver terminates without that marker. It
first runs the verified-input old-leak positive control, then all nine external A/B
pairs. External commands retain `--local --track sharingan-1 --hands 20004 --lanes 8
--seed ab-20004`; use only the immutable `arena-odds`, the verified private C path,
`ARENA_PASSIVE_ODDS=0|1`, medium sizing and the original buckets. Require all planned
hands, zero forfeits/crashes/illegal actions/timeouts and identical deal digests in
both arms. Run as `ubuntu` with a constructed environment containing no credentials;
cap Rayon, Tokio, OpenMP and BLAS to one worker per lane so the eight native/bot pairs
stay within the host allowance. Stop on any significant external regression.

After those pass, explicitly run the ignored `passive_odds_on_menu` fixture with
`ODDS_TEST_MODEL` pointing to the verified C; then run both 20k-deal LBR arms with
seed 5, eight workers, same model and native executable. The one-deal ABR placeholder
in LBR outputs is not interpreted. Only after those no-regression gates, launch the
predeclared fresh 1M / 15-GB / seed-8 attacker using 11 workers, save native weights,
verify buffered/direct archive hashes, require exact enabled reload at seed 161804,
and perform both cross-policy A/B seeds. The queue stops on any replay failure or
significant AIVAT/raw defender regression. Final-model and normal-strength checks are
still separate mandatory gates; this queue cannot promote code. Every command,
constructed environment and output is recorded under `hardening-08/`.


**Frozen C A/B completed (candidate still uncommitted):** both preregistered stages
passed. Each row is 100,000 independent duplicate deals per seat/arm (600,000 hands),
with the same frozen policy and deal seed in both arms; units are attacker mbb/game,
95% CIs.

| frozen attacker | seed | correction disabled | mixed-passive correction enabled |
|---|---:|---:|---:|
| 3M / 35 GB | 271831 | +2,355.9 ± 106.0 | +441.3 ± 100.6 |
| 3M / 35 GB | 314161 | +2,191.4 ± 106.4 | +267.8 ± 101.1 |
| original 1M | 271831 | +499.9 ± 94.9 | −787.3 ± 91.2 |
| original 1M | 314161 | +408.1 ± 95.7 | −901.6 ± 91.9 |
| iteration-6 adaptive 1M | 271831 | −1,342.5 ± 52.3 | −1,378.1 ± 52.4 |
| iteration-6 adaptive 1M | 314161 | −1,349.2 ± 52.2 | −1,385.2 ± 52.3 |

The primary 3M improvement has non-overlapping intervals on both seeds; it still finds
significant profit. The older adaptive attacker finds neither significant improvement
nor regression. All declared AIVAT/raw no-regression gates passed. Artifacts:
discovery-gates.json, holdout-gates.json, frozen-gates.ok under hardening-08/.
Exact command template (driver itself is already nice 10; do not add another increment):

~~~sh
taskset -c 0-10 hardening-08/exploit-odds --compact \
  --target models/sharingan-1-verified.plcp --sizing medium \
  --buckets /srv/sharingan/models/buckets-200 --players 6 --iters 0 \
  --threads 11 --mem-gb 15 --learner-sizing adversarial \
  --load-learner hardening-02/learner-3m.bin --seed 2 \
  --eval-seed 271831 --eval-deals 100000 --no-passive-odds \
  --out hardening-08/discovery-3m-base.json
~~~

The enabled arm substitutes --passive-odds; holdout substitutes 314161 and fresh
output names. Original 1M uses its archived weights/training seed 1, old adaptive uses
its archived weights/training seed 7. Every individual argv and constructed environment
is in its *-command.json; identities match the binary/model/learners recorded above.

**External gates and actual-model on-menu fixture passed:** the old-leak positive control
is **+5,381.8 ± 513.9**, with its lower bound above the registered +1,000 threshold.
Each external A/B is 20,004 hands, eight lanes, seed ab-20004, identical deal digests,
zero forfeits, crashes, illegal actions or timeouts:

| attacker/control | disabled | enabled |
|---|---:|---:|
| ctl_quickstart_offline | −596.4 ± 135.2 | −628.4 ± 136.7 |
| ctl_tag | −465.6 ± 118.5 | −463.8 ± 118.3 |
| sombra | −419.6 ± 161.6 | −434.3 ± 159.9 |
| bracket edges | −4,173.8 ± 573.4 | −4,367.7 ± 577.8 |
| minimum bet | −5,983.4 ± 637.3 | −6,802.6 ± 640.5 |
| minimum-raise war | −8,937.9 ± 735.9 | −9,373.0 ± 742.5 |
| overbet | −661.6 ± 197.2 | −672.2 ± 196.2 |
| random size | −5,745.0 ± 485.7 | −6,333.8 ± 481.9 |
| tiny shove | −1,801.8 ± 353.0 | −1,921.1 ± 352.0 |

No pair has a significant defender regression. The offline quickstart control uses its
recorded local version and is not interchangeable with an earlier differently configured
quickstart estimate. The previously ignored native-model fixture was explicitly run:
20,000 strict-menu replay hands, **362,440 bit-identical distribution queries**,
then 100,000 duplicate deals/arm. Both arms are exactly
−3,553.8642 ± 105.4662 for its deliberately weak uniform-menu control, including identical
raw winnings −3,633.8323 ± 110.0033. This checks preservation, not playing strength.
Artifacts external-gates.json, strict-control-c.json, and command/log files.
The queue is now running clean-input paired LBR; fresh adaptive, final-S2 effects,
external final-model checks and the 4M normal-strength gate all remain mandatory.
**No policy change is kept or committed yet.**

**Additional LBR reference gate:** before the verified-input baseline finishes, stop
only the owned broad-queue parent PID 891541; the existing eight-worker LBR child
950127 keeps running. Nice-10 verifier sharingan-verify-clean-lbr-v1.py (PID 986714),
SHA b8228af64dae0a61c1151ca59b7534996d1bf654160090acd0e7c7aa39198dc4,
waits for that same child to finish, then compares the complete LBR result with the
original seed-5 reference, plus the one-deal placeholder and normalized tracking options.
The placeholder is checked only for replay, never interpreted as a strength estimate.
If all agree, record clean-lbr-baseline-replay.ok and resume only that owned parent;
otherwise retain both results and hold the queue for investigation before any candidate
score is trusted. No live process is signaled. This closes a missing reference check
in the broader driver; its sample size, candidate code and other thresholds are unchanged.


**Canonical LBR reference passed:** all three checks are exact: the complete weak/strong
LBR object, the ignored-for-strength one-deal placeholder, and normalized tracking options.
Strong remains **+841.4236 ± 161.7408**, weak **+330.0176 ± 164.7779**, each over 20,000
duplicate-deal clusters (120,000 hands/configuration), raw winnings. This is a replay
on verified input, not another independent seed or extra statistical sample; retain the
original CIs. The verifier resumed the owned parent, and the candidate LBR arm is now
running. Artifacts clean-lbr-baseline-replay.json/.ok and clean-lbr-replay-v1.log.

**Remaining-profit localisation:** run the existing --audit-handoffs wrapper against the
same frozen 3M attacker at 271831, 100,000 deals/seat in each arm. Four workers on CPUs
10–13 at nice 10 share only the verified C target with the eight-worker LBR job.
The wrapper must preserve every exploitation statistic, learner miss rate and target
diagnostic, and its 600,000 hand/payoff totals and per-reason counts must reconcile.
Treat payoff groups only as observational raw attribution; they exclude missing-infoset
fallbacks and cannot establish causal loss or a CI. The first validator incorrectly
aligned serialized reason counts (the target array starts at reason 1, the audit at 0).
It stopped after measuring baseline despite all other checks passing. Preserve that
failed validator/log. Version 2 aligns audit indices 1–5 with the complete target
array, verifies the already measured baseline without rerunning it, then measures the
enabled arm once. No policy or match data changes. Drivers/SHAs:
sharingan-odds-handoff-audit-v1.py, 9b013bfbc1e20ed7069e6a22ef10a51543acab6f1bd75d4b46f450bf4c8f0fce,
and sharingan-odds-handoff-audit-v2.py (PID 962202),
68ee5229972bddef0dd95d277452115f3d637292def1026292a979a8291ce0c6.

**Audit completed:** both corrected validators pass all six checks. Baseline handoffs
occur in 4,670 / 600,000 hands (0.7783%); enabled handoffs in 4,692 (0.7820%).
Their raw-payoff contributions, divided by *all* hands, are +9.12 and +9.19 mbb/game.
The no-handoff groups contribute +2,366.99 and +458.81, respectively; the sums exactly
reconcile to raw totals +2,376.11 ± 124.63 and +468.00 ± 122.15. The conditional groups
have no independent-deal CIs here and membership changes between arms. This is evidence
about where the observed payoff occurs, not a causal ablation of handoffs. Most observed
profit, and most of this correction's observed change, occur without a shadow handoff.
Artifacts audit-3m-{base,odds}.json and corresponding -gate-v2.json; no extra deal seeds
or policy changes were selected from this diagnostic.

**Final-model frozen A/B queue preregistered:** s2-odds-frozen-v1.py,
SHA 98ca8c754599fcf013ea968af918240d4a36015fff7ac5dcaef3a0a3c6bb6e2d,
waits for the independently verified 3M final-model baseline above. Copy only the
6,812,816-byte candidate executable to the task-owned extra worker; its readback SHA
is b9d8e27a…, identical to the tested original-host binary. Check current-boot model
certificate/file metadata, both native learner full hashes and relocated manifests,
and bucket archive hash. Before A/B, the disabled candidate binary must exactly replay
the public-S2 1M reference at 271829 and both newly measured 3M references at 271828 and
314159 (all exploitation statistics, miss rate, normalized tracking options).
Then test frozen 3M and 1M, each baseline followed by enabled candidate, at discovery
**271831** and holdout **314161**, 100,000 duplicate deals per seat/arm, all six seats.
Require a disjoint-CI primary 3M improvement on both seeds and no AIVAT/raw defender
regression for either policy. Use six workers on CPUs 4–9 at inherited nice 10, explicit
--no-passive-odds / --passive-odds, --iters 0, medium sizing, original training seed
2 or 1 and unchanged archived weights. Exact argv/environment/identities go beside each
fresh result under the worker's hardening-08 directory. This queue cannot promote code:
the new adaptive attacker, relevant final-model external controls, and 4M head-to-head
remain required. No final-model candidate A/B result has been observed.

**Original-host final gates preregistered:** sharingan-odds-final-host-v1.py,
SHA 1e9ed9de8345c02174727bc11933c65964fc99d97f1db3a5dd24a91d1b86ec1c,
requires the completed C broad/fresh-adaptive gate, completed diagnostic audit, and
independent final-model frozen-gate proof. A Mac-only watcher relays only that small
proof after checking it, never an SSH key. An independent frozen failure or a failed C
gate stops the final queue. Before using the original-host S2 file, require its complete
buffered and O_DIRECT hashes to equal 0f57b296… with unchanged metadata; then exactly
reproduce both independent S2 1M samples and the independent 3M discovery sample using
the disabled candidate binary. Any mismatch stops for input investigation.

If those pass, test the newly trained seed-8 attacker on S2 against both arms at
161804 and 271832, 100,000 duplicate deals/seat/arm, and reject any significant
AIVAT/raw defender regression. Repeat the old-leak positive control, all nine external
A/B pairs at 20,004 hands and seed ab-20004, the actual-model strict-menu fixture, and
both 20k-deal weak/strong LBR arms at seed 5. The external runs remain solely under
ubuntu on the original host with constructed environments, one native/bot worker per
lane, no forwarded credentials and no live-service interaction.

Only then run the existing three-format head-to-head harness with S2 as A and verified
private C as B. Both arms use seed **1**, medium sizing, **4,000,000 deals per format**,
12 workers on CPUs 0–11, nice 10, no --warm. Baseline explicitly disables both sides;
candidate enables S2 only and keeps C disabled. A 200-deal mechanical smoke runs first;
its noisy scores cannot tune the fixed 4M sample size. The baseline 3+3 interval must
overlap the documented **+5.09 ± 1.16** before the candidate run starts, otherwise stop
and investigate the reproduction. In every format reject a significant AIVAT or raw
loss of S2 strength (account for the reversed first side in 1B versus 5A). Record
all argv/environments, input proofs, output and gates under hardening-08. These are
sequential private offline jobs; the prescribed head-to-head is the only job on that
host that maps both models. Even all passing markers cannot deploy or commit policy
code; default promotion still requires source review and revalidation.

**Transfer accepted and final-model baseline started:** the bounded relay completed,
and the destination's full compressed SHA matches bb87d61c…. Decoding, full native
hash and separate native readback all match 1706c51f… at 29,581,123,222 bytes.
The source manifest hash and relocated policy configuration passed. The predeclared
3M final-model baseline is now evaluating its first seed; there was no full blueprint
copy on the Mac. The conditional original-host final queue is PID 965387, and Mac proof
observer PID 13856 (script SHA c84d3fc49b742a4f3f02d2594e0a2e06af0afccfafab3e82dd986a1521a86da4).
All source/model/policy gates above remain in force.

**Completed measurements and wrap-up checkpoint (2026-11-04):** all values below
are attacker mbb/game with 95% deal-cluster CIs. The frozen final-model A/B passed
its three exact disabled-policy reference replays and both registered seeds:

| final SHARINGAN-2 attack / seed | disabled | mixed passive odds |
|---|---:|---:|
| frozen 3M / 271831 | +2,543.1 ± 105.7 | +428.1 ± 100.0 |
| frozen 3M / 314161 | +2,382.6 ± 106.9 | +251.6 ± 100.8 |
| frozen 1M / 271831 | +715.3 ± 94.0 | −700.0 ± 90.0 |
| frozen 1M / 314161 | +632.4 ± 95.2 | −796.2 ± 91.1 |

Each row/arm uses 100,000 duplicate deals per seat / 600,000 hands. Both primary
3M improvements have disjoint intervals; none of the four pairs has a significant
raw or corrected defender regression. Artifacts discovery/holdout-gates.json and
frozen-s2-gates.ok under the independent worker's hardening-08 directory.

The fresh seed-8, 1M-iteration attacker also completed its input seal and exact
reload. On C at evaluation seeds 161804 / 271832, disabled scores are
−445.5 ± 62.2 / −478.5 ± 62.3, versus enabled −716.8 ± 61.5 / −748.6 ± 61.6.
On verified original-host S2 the same frozen attacker scores
−197.8 ± 60.4 / −232.1 ± 60.5 versus −514.8 ± 59.6 / −554.2 ± 59.6.
Each is 100,000 duplicate deals per seat; all raw and corrected no-regression gates
pass. C LBR weak: +330.0 ± 164.8 → +321.2 ± 164.7; strong:
+841.4 ± 161.7 → +727.2 ± 163.4, 20,000 duplicate clusters/configuration.
Those LBR differences are inconclusive.

Original-host S2 full buffered/direct hashes and all required exact reference
replays passed. All nine final-model external A/B pairs completed without errors
or significant defender regression at seed ab-20004 / 20,004 hands per arm:

| external attacker | disabled S2 | mixed passive odds S2 |
|---|---:|---:|
| sombra | −390.4 ± 155.5 | −429.7 ± 153.7 |
| quickstart offline | −555.4 ± 134.3 | −575.6 ± 134.4 |
| tag | −382.1 ± 115.3 | −378.8 ± 114.9 |
| bracket edges | −4,367.4 ± 584.7 | −4,395.3 ± 588.2 |
| minimum bets | −5,933.4 ± 639.4 | −6,745.8 ± 640.7 |
| minimum-raise wars | −8,949.7 ± 738.9 | −9,393.6 ± 745.2 |
| overbets | −710.0 ± 197.7 | −723.5 ± 196.4 |
| random sizes | −5,650.7 ± 479.7 | −6,324.9 ± 471.1 |
| tiny bets / shove | −1,784.4 ± 352.3 | −1,862.1 ± 352.4 |

The user then requested wrap-up. The owned final-host process group was stopped
during the enabled final-model LBR arm; its completed baseline and all prior files
remain under ~/fix/hardening-08, with user-wrap-up-checkpoint.json. The required
final LBR pair and 4M-deal normal-strength gate are incomplete. Additional native
near-all-in S2 validation in iteration 10 is also incomplete. **No iteration-8
policy change is kept on main or committed.** Its exact source is preserved,
uncommitted, in the sibling sharingan-passive-odds worktree on branch
hardening/passive-odds, plus the immutable remote source archive/binaries.
This is an unfinished experiment with substantial target improvements, not a
validated strength improvement or a completed hardening goal.

### Hardening iteration 9 — guard terminal scaled-engine replay in the inference API (2026-11-04, preregistered)

**Hypothesis and reproducer.** The API's real/scaled chip conversion can round a
non-all-in raise into a scaled all-in. Once all players call, the scaled hand can
end while the real players each retain a chip. A later real event unconditionally
calls to_scaled before the desync/terminal guard; that calls State::legal on a terminal
state whose actor is 255. A task-owned external diagnostic, using a synthetic always-call
blueprint source, reproduced a valid six-player request: 500/1,000 blinds, 100,000-chip
stacks, button 5, seat 2 raises to 99,999, all five others call, then seat 0 puts its
last chip in on flop Qh Jh Th with hero seat 1 holding Ah Kh. The real hand is ongoing;
the scaled one is terminal. The diagnostic **panics** at the bounds check in
nlhe/src/lib.rs:359. The equivalent 50/100-blind request calls correctly.
No live API request was made. The isolated release binary aborts; a server built with
unwinding catches decision panics, so this does not establish the live service's process
behavior. Core dumps were disabled for the diagnostic. This is independent of model
weights and the uncommitted passive-odds correction.

**Smallest candidate and gates.** In a separate worktree based on current main, move
scaled-action conversion inside the existing synchronized, correct-actor, nonterminal
guard. Preserve the real action validation and existing legal fallback; do not change
the unrelated poker-policy candidate. Add mandatory, model-free regressions for the
continued real history, terminal scaled state, legal fallback and exact-scale control.
Measure the same preconstructed legal request set against before/after replay: baseline
panics versus guarded legal responses, using an unwind build to count each case safely;
report binomial 95% intervals. Require zero new errors or panics on legal exact-scale
controls and equality of their states/actions/probabilities. Run the requested release
tests and builds remotely (including serve's model-free tests), bounded at four workers
alongside the existing eight-worker C LBR job. This API-only guard cannot change the
arena/bench/pluribus policies; no normal-strength gain is claimed. The existing API
desync fallback remains check/fold, so eliminating this panic will not establish poker
robustness for rounded-chip histories. Keep and push only after the registered checks
pass; never deploy a server.

**Registered source and exact A/B sample:** worktree hardening/api-scaled-terminal,
base 8d02c87891268c24184c4845800603eb891aad3c. Only serve/src/replay.rs changes:
candidate SHA f128f45662fef62c3ce963959a3516f06386faa338fdd0efb3e04e52a08cc6b5,
baseline b48eb51a66c51f5932cc3bd4a3c95943dfc7be018ff72e52f6bf6ba10ab70036,
patch 385e05b91d48bd906bf5090c898b15fc77c6d13b4e4ee388bd8e1df1f66e874e.
The source registration hashes all 187 Rust/Cargo inputs. A separate original-host
base tree differs from the guarded tree only in that one source file.

Harness SHA 06efe12b728dd5263823637fbb46de3eef620272a33baad63e611ae7799065c4:
generator seed **2026110409**, **512** rounded-chip requests with random big blinds
1,000–1,000,000, button rotations and hero positions, plus **512** legal exact-scale
controls with blinds from {2,4,5,10,20,25,50,100}. Each has 100-BB stacks and the
near-all-in/continued-flop construction above. Build identical harness source against
clean base versus guarded serve, in the unwind profile; catch request panics to count
them without ending the measurement process. Require all constructed rounding cases
to expose the old panic, zero guarded panics/errors/illegal responses in both groups,
and identical normalized control responses (action, probabilities, legality, source,
reason, notes, street and model ID; timing/request ID excluded). The two crash-rate
Wilson 95% intervals must not overlap. These are synthetic request-sample intervals,
not a poker EV or exploitability estimate.

Driver sharingan-api-terminal-validation-v1.py (PID 976836), SHA
4065cc3f4cbb02adc58734ff3fc8d2ea7a6f0397b4414e65af24877a9e24d9cc,
records every argv/environment, copied executable identity and output under
~/fix/hardening-09. It uses nice 10, four build/Rayon workers on CPUs 10–13,
offline Cargo, and its own target directory. After A/B it runs all requested release
tests (also pluribus and serve) and all six builds on the clean main-based guarded
source. Model-dependent serve fixtures may skip when their tiny model is absent;
do not count those as exercised model tests. The four added unit regressions require
no model and must execute. No source changes or new A/B numbers have informed this
registration.

**Outcome: kept as an API availability fix.** The registered before/after sample passed
every gate:

| request group | before | guarded replay |
|---|---:|---:|
| rounded-chip cases | 512 / 512 panics | 0 / 512 panics; 512 legal responses |
| exact-scale controls | 0 / 512 panics | 0 / 512 panics; all normalized responses identical |

The rounded-case panic-rate Wilson 95% intervals are **[99.2553%, 100%]** versus
**[0%, 0.7447%]**, disjoint. These intervals describe the fixed seeded synthetic request
sample, not a prevalence estimate over real users or a poker EV measurement.
The guard has zero request errors and zero illegal answers in either group.
Only serve/src/replay.rs differs between the clean before/after source trees, and all
187 registered source hashes still match after validation. The implementation moves
conversion into the existing guard; real-history validation and the existing legal
fallback stay intact. This does not repair the fallback's poker weakness.

The requested release tests plus pluribus/serve and all six builds passed. Cargo
reports **179 passed / 37 suites, zero failed, zero ignored**. Of these, **17**
model-dependent API integration cases returned early because the optional tiny blueprint
was absent; those are not exercised model tests. The remaining **162** executed,
including all four added mandatory, model-free replay/Core regressions. Exact commands
use the registered clean environment and four workers on CPUs 10–13:

~~~sh
cargo test --release -j 4 --offline -p nlhe -p bench -p search \
  -p sharingan-arena -p exploit -p pluribus -p sharingan-serve -- --test-threads=4
cargo build --release -j 4 --offline -p sharingan-arena -p bench -p exploit \
  -p search -p sharingan-serve -p pluribus
~~~

Both are run under the nice-10 driver's affinity with its separate target directory.
The identical A/B harness was built with --profile serve --offline -j 4 and run
as api-base / api-guard. Executable SHAs are
1e2c16ec643a1ef1af79939b6fbbfd47b9c3a3b50c4c48545cb2a0cfdea3196b and
2e3251215610d14c49fddde427550acab65588b1fa23dfc27c71ff035c73d13d.
Artifacts api-ab-gates.json, source-differences.json, validation-summary.json,
validation.ok, both complete request-output files and all command/build/test logs.
The arena/bench/exploit/search/pluribus policy sources are unchanged by this fix;
the separate iteration-8 poker-policy candidate still requires its remaining measured
gates. No service, model, deployment, live request or website was changed.

### Hardening iteration 10 — test forced folds after API chip rounding (2026-11-04, preregistered)

**Hypothesis.** Iteration 9 prevents the replay panic but intentionally retains the
API's check/fold fallback. At 500/1,000 blinds and 100,000-chip stacks, a raise to 99,999
can round to a scaled all-in. Callers retain a real chip although the scaled hand ends.
An attacker may then win a very large pot by betting its last chip into targets that
automatically fold. Test this on actual Core::decide before changing any policy.

**Baseline finder:** a trusted Rust attacker raises to one chip below its legal maximum
preflop whenever that non-all-in raise is legal, folds expensive preflop calls if it
cannot make that raise, and shoves its remaining chips postflop. It uses no opponents'
private cards. Play against five stateless API players, no search, all six attacker
seats, **3,334 independent duplicate deals / 20,004 hands**, seed **2026110410**.
Also run a same-seed, same-game control leaving **10** chips, exactly one scaled chip,
instead of one. Require the one-chip attack's lower 95% bound above +1,000 mbb/game,
positive API desync exposure, zero API errors and all returned actions legal. The
10-chip control must have zero API desync exposure; report its score without assuming
that its poker strategy loses.

The target player calls the real Core with generated legal full-history requests and
seeded API sampling. It does not substitute a policy reconstructed from rounded response
probabilities. API targets are marked **non-exposing**, so AIVAT corrects chance nodes
and the deterministic exposing attacker's actions, not API target decisions. Also report
raw duplicate winnings and deal-cluster CIs. Disable API JSON logging; every hand is
generated offline, with no live request, token or participant data. The baseline uses
the clean main-based iteration-9 guarded source, all 187 Rust/Cargo hashes checked,
and the same verified private C file already reproduced by the frozen/LBR references.
One model per host; four nice-10 workers on CPUs 10–13 alongside the existing C LBR job.
Artifacts go in ~/fix/hardening-10.

Harness source SHA 30396229864ac8dc92ee3c9d12f26d77f7d9a91b3f5e75cf2bc411555bff4cf1;
immutable driver sharingan-api-rounding-baseline-v1.py checks inputs, builds offline in
the unwind profile, copies an immutable executable, and records all commands and hashes.
The exact-grid control is a separate attack, not extra independent data to pool with the
primary. Stop and investigate if the mechanics gates fail; do not promote a fix from
the royal-flush point example alone.

**Baseline outcome, before any policy repair:** the actual guarded API target loses
**14,283.3 ± 910.5 mbb/game** to the one-chip attacker; raw duplicate winnings are
**14,079.5 ± 873.6**. There are 114,814 target decisions, 3,442 desync fallbacks,
zero API errors and zero illegal actions. The exact-grid ten-chip attacker also wins:
**4,117.2 ± 771.4**, raw **4,269.5 ± 1,035.4**, over 115,705 target decisions with
zero desync fallbacks, errors or illegal actions. Both use the registered 3,334 deal
clusters / 20,004 hands. The exact-grid result is a separate profitable attack;
neither its profit nor the difference between scores establishes a causal estimate
of the rounding leak. These API scores correct chance and attacker decisions only.

Driver sharingan-api-rounding-baseline-v1.py SHA
016312718267ab5c6bad4e15bee134d88558820460447377248a62040e513f5a
finished with baseline-gates.ok. Immutable executable SHA
d9dac876495fe7a604d260d0b0c05b61f27bd7116c58d97bdea10980324aa518;
outputs positive-left1.json and exact-grid-left10.json under ~/fix/hardening-10.
No policy repair has yet been measured or retained.

**Additional iteration-8 gate, registered before native results:** the profitable
exact-grid API attack motivates testing the same trusted near-all-in strategy
against native CompactPolicy. At canonical 50/100 blinds it leaves one chip,
uses discovery seed 2026110410 and holdout 2026110411, and plays 3,334 duplicate
deals / 20,004 hands per arm. Compare the immutable mixed-passive-odds candidate
disabled/enabled, exposing native target probabilities for full AIVAT. Require no
significant raw or corrected defender regression on either seed; retain all prior
iteration-8 gates. Confirm on final SHARINGAN-2 before retaining that candidate.
First require exact replays of both API baseline outputs and diagnostic counts
using the extended harness; investigate any mismatch before interpreting native
numbers. The harness source SHA is
901583d20142a65904378da685b01ebb0b7489d1b349544df53560df10449f60.

Placement amendment before native measurements: build the extended harness only
on the original host (four nice-10 workers), then transfer the trusted executable
to the existing private validation machine. Run C then SHARINGAN-2 sequentially
there, with current-boot input hashes and the API replay gates, while the original
host completes its final-model validation. Do not start another C inference job
alongside that host's final-model stage. All sources, commands, hashes, outputs
and gates must be preserved; this changes placement, not the registered samples.

The validation machine's boot ID changed from 3d1e07ff-d2e7-415f-ac55-7c7013d92277
to ac5b299f-d3a3-40e6-8a36-c065fbca06b4 before the additional native run. The v2
input guard rejected the old-boot S2 certificate before hashing or inference;
its traceback is retained, and it produced no new poker measurements. The v3
driver must hash C in full, replay both original API results exactly, measure
the registered native C pairs, then hash S2 in full and exactly replay the frozen
1M reference (271829, 100,000 duplicate deals per seat) before its native pairs.
Keep all sources/logs in separate v2/v3 directories. The extended executable
SHA is 3a9c570e948c497102d28d2259402c26184aaed1ce450f1eed1c00aa3216bb97;
the original-host compile-only stage opened no model and finished successfully.

**Additional native C outcome:** v3 full C verification matched 2a4484f6… on boot
ac5b299f…, and both original API baseline statistics and diagnostic reports replayed
exactly. At canonical 50/100 blinds the native one-chip attacker scores
**+3,964.7 ± 788.3 → +954.4 ± 740.0** at discovery seed 2026110410 and
**+3,771.9 ± 806.3 → +253.3 ± 753.3** at holdout 2026110411, disabled →
mixed-passive-odds candidate. Raw scores are +3,922.5 ± 1,069.5 →
+918.9 ± 1,045.5 and +3,757.1 ± 1,077.7 → +119.6 ± 1,042.2.
Both corrected improvements have disjoint intervals and neither pair regresses
under the raw check. The discovery candidate remains significantly profitable.
These are 3,334 clusters / 20,004 hands per arm, with fully exposed native
probabilities; do not pool or directly compare them with the unexposed API scores.

Driver v3 SHA 7c3f17e6f19f15f28463470df20de7a48d43d3497ae88ffcfbf67fdd1abc570b
and every argv/output are preserved under the validation worker's
hardening-10-native-v3 directory. S2's full hash then passed, but a further machine
restart interrupted the required frozen-reference replay (no result JSON or gate).
The later boot is 489d7160-c395-4213-8dd6-3fbcc988dbf1. There are **no native S2
near-all-in results**; do not reuse an old-boot certificate to resume inference.
All old logs remain intact. No replacement run was started at wrap-up.

The native measurement command is the immutable extended executable followed by
MODEL BUCKETS OUT 3334 SEED 100 1 4 native false (disabled), or true (enabled),
at the two registered seeds. API replay substitutes 1000 for the big blind,
1 or 10 for chips left, api for the mode and false for the correction. The v3
driver records absolute argv, explicit environments, input certificates and
readback identities beside every result. A small archive of all v3 JSON, logs,
source and completed markers is preserved at
~/fix/hardening-10/native-v3-wrap-up-checkpoint.tar.gz, SHA
7430e36ce5d77a39e0bee7c3a31fd8ce86d332d8c7a5209ba6091abba4ee56c5;
the local copy is in the sibling checkpoints/wrap-2026-11-04 directory.

The proposed real-state API fallback was prepared in the sibling
sharingan-api-real-fallback worktree, branch hardening/api-real-fallback. Its source
archive SHA is 4d787a7e6d4cdadea1bc74a859a029985ccf468ecf06144c2c5b13822e393682.
It changes only the API's missing-distribution action/probability branch and
extends the model-free royal-flush regression. **It has not been built, tested,
measured, committed or deployed**, and is not part of main. Preserve it as an
unverified candidate, with all registered A/B, controls and release gates still
required. The autonomous loop is stopped at the user's wrap-up request.

**Contingent smallest repair and planned gates:** if the positive control establishes
the leak, replace only the API's missing-distribution check/fold branch with the existing
bench fallback evaluated on the **real-chip state**, board and hero cards. A scaled
action must not be mapped back to real chips twice. Return the actual deterministic
fallback action with probability one when probabilities are requested. This remains an
equity-versus-random heuristic, not a range-aware solution.

Require significant primary A/B improvement with the same frozen attacker and discovery
deals, then repeat both arms at reserved seed **2026110411**, the same 20,004 hands/arm.
Report raw and corrected CIs; reject a significant raw or corrected regression. Keep
the 10-chip control identical. Add frozen attackers leaving 2, 5 and 6 chips as adaptive
boundary checks on both seeds, and require no significant defender regression. Run
the actual existing sombra/probes/quickstart/tag controls through a validated SDK bridge,
20,004 hands/arm, and confirm the primary effect on verified final SHARINGAN-2.
Sombra remains restricted to ubuntu on the original host. Canonical 50/100-blind
requests must preserve states, seeded actions and probabilities exactly. Run required
release tests/builds, including the mandatory model-free API regressions. Keep this
experiment isolated from iteration 8; do not commit a strategy change before its gates.

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
* Our blueprint beats SvanBot head-to-head in both seatings, also at SvanBot's full live budget.
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
