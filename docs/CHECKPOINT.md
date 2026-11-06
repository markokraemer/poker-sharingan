# Research checkpoint — November 2026

Development and remote experiments were stopped at the user's request. This is
an Apache-2.0 research project for local training, inference and evaluation.
No hosted service is required or promised.

## Retained state

The previously validated action-translation synchronization and real-state API
fallback fixes remain in the source. No new policy change was accepted during
wrap-up. Model weights are unchanged. SHARINGAN-1 is C@2e10; SHARINGAN-2 is the
final C@1.83e11 checkpoint. Both use medium sizing and 200 postflop buckets.

## Known exploitation

Both blueprints have confirmed profitable attacks. A frozen 3M-iteration attacker
scores +2,466 ±75 mbb/game against final SHARINGAN-2 and +2,307 ±75 against
verified C@2e10 (duplicate + AIVAT, 95% CI; no search). These are lower bounds
from specific attackers, not bounds on the largest possible loss.

The experimental nearest-all-in handoff improved some frozen attacks but failed
against newly adapted attackers. Against final SHARINGAN-2, two held-out seeds
returned +265.413 ±44.197 and +249.430 ±43.855 mbb/game for the attacker.
On the latter seed the earlier snap variant returned −406.322 ±43.170.
Nearest was rejected; it is not a default or a validated release.

A main-only equity memo optimization also failed its timing gate: geometric
mean speedup 1.488, 95% interval [0.517, 4.285]. It was not retained.

## What remains incomplete

Final normal-strength and remaining full evaluation gates were stopped.
Experimental snap/odds variants, optional attacker fallback modes and the
locked-river fallback prototype are not promoted by this checkpoint. Completed
unit tests or favorable subsets do not replace the missing gates. No claim of
unexploitable play or universally stronger search is made.

See [results](RESULTS.md#action-translation-leak-and-fix-nov-2026) for hypotheses,
measurements, rejected experiments and provenance. Operational receipts were
saved separately from the public source; untracked research material was not
added to the release. Do not resume automated jobs without an explicit request.
