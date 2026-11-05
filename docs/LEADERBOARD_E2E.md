# Live leaderboard E2E — 2026-11-05

The user requested removal of the old sombra leaderboard entry and fresh operator test bots.
The supported admin API deleted player p_1ab9e84e4efb and its four finished benchmark runs;
there were zero associated matches. The row was deleted, not hidden by the website.
No participant contact data was read or logged.

Four trusted Python examples were uploaded through the existing hosted runner, using
operator service authentication (no invented contact/profile fields). Each job ran
in its normal isolated, network-disabled worker sandbox on fresh duplicate deals.
The runner submitted its signed result to the arena, which accepted it as verified.
Each job played 20,004 hands, recorded zero forfeits, and successfully deleted its sandbox.
The existing services, binaries, images and model files were not rebuilt or reconfigured.

| bot | live score, mbb/game ± 95% CI | public run |
|---|---:|---|
| e2e-quickstart-c3ddb3 | -316.2 ± 95.2 | [w_f47864ab1c68](https://arena.sharingan.poker/v1/benchmarks/sharingan-1/runs/w_f47864ab1c68) |
| e2e-tag-923112 | -284.6 ± 120.5 | [w_ac44311ffc83](https://arena.sharingan.poker/v1/benchmarks/sharingan-1/runs/w_ac44311ffc83) |
| e2e-min-raise-80e214 | -3,631.3 ± 361.4 | [w_d8979fc1eb1d](https://arena.sharingan.poker/v1/benchmarks/sharingan-1/runs/w_d8979fc1eb1d) |
| e2e-near-all-in-d25e00 | -2,780.6 ± 475.7 | [w_12a3b601c03c](https://arena.sharingan.poker/v1/benchmarks/sharingan-1/runs/w_12a3b601c03c) |

Track ID remains sharingan-1. All four report blueprint fingerprint f22c4ba8c43c0720
and fresh deals. These are operational E2E results against the currently deployed
worker, not measurements against the independently validated fixed offline build.
They use different fresh samples and must not be pooled with earlier experiments.

## Deployed source version remains unverified

The near-all-in result differs sharply from the canonical fixed-build experiment.
A separate 600-hand, unranked diagnostic inspected only the public worker executable
from inside its own sandbox, printing its hash and two literal-presence checks.
It completed verified with zero forfeits and sandbox teardown. No model or secret
file was accessed by the bot.

Deployed worker SHA256:
4610eed5f52c068f3892d4cb2ae36bfccd8d47b8aa81014f1364ac837fac0918
(66,510,520 bytes). Both pre-sync-fix and keep_sync literal markers are absent.
The validated offline arena binary SHA256
43a141acf05e71fd8dc64c966b9fc85700bd63022682c690a52b1d2b0fbdb6f7
(62,068,256 bytes) contains keep_sync, but also lacks the pre-sync-fix literal.
The marker diagnostic therefore failed its intended positive control: literal
absence cannot establish that the deployed source lacks the fix. An initial
version-mismatch interpretation was withdrawn before retaining this report.
Both identities and the failed/inconclusive diagnostic are preserved.

The unchanged blueprint fingerprint identifies neither the complete model contents
nor the policy implementation. Do not use these scores to claim that the fixed
offline policy is deployed or that sombra would now lose to hosted workers.
Sombra scored −419.6 ± 161.6 against the fixed C offline build, but was not rerun
against the live deployment in this task. The difference in near-all-in scores
remains unexplained; no policy improvement or regression is inferred from it.
No additional strategy experiments or deployments were started.

The user subsequently requested the original sombra submission be rerun. That
separate hosted run completed at +4,760.7 ± 491.7 over 20,004 hands, verified with
zero forfeits. It is a new operator row, not the restored historical entry. See
[the original-submission rerun](RESULTS.md#original-sombra-submission-rerun-2026-11-05)
for its exact artifact identity and the separate fixed-policy checks.

## Reproduction and evidence

The committed sources are in sdk/python/examples/e2e_*.py. Ordinary public submissions
can use the SDK submit command with the caller’s own contact/profile and bot credentials.
These operator runs instead sent POST /v1/jobs with worker mode, the source tarball,
track sharingan-1, hands 20004, lanes 2, and stable idempotency keys. No credentials
were printed, committed or copied to a new machine.

Exact operator scripts, sanitized job receipts, artifact/source hashes and gate markers
are preserved under ~/fix/leaderboard-e2e-2026-11-05 on the existing work host.
A sanitized local copy is in the sibling checkpoints/leaderboard-e2e-2026-11-05.
The hosted-e2e.ok and near-allin-e2e.ok markers require successful signed runs,
zero forfeits, exact run/job agreement, current fingerprint agreement, sandbox teardown
and visibility in the public leaderboard API. Browser verification additionally checked
all four rendered rows, absence of sombra and the old score, and both sorting controls.

| source | source SHA256 | artifact SHA256 |
|---|---|---|
| e2e-quickstart-c3ddb3 | 9c715bf4ec638523944523e7a2747eaad61a5eb6f58b317b0d56ecb07f832df1 | 78ef1e6aeb9de55ce72a6d2ae1504bc4a44d58b4755551973888dd9dc8c14e5d |
| e2e-tag-923112 | 2fc47299c326e48b16c3c8bad3578acea2b161461b25d7825eb48bec682315b2 | 444c424fa6772571b82698e17d6a6d509dc574078d414911b981428ea11e046f |
| e2e-min-raise-80e214 | 109326ca6f6ddc164ac53afa0fc9d0f101ee4aca0a62a7e8bd8e14fb0b7bf941 | 400d3f2afb439f0729e334f05f79e235beb84c82b8397e4abb08e8e109ae69f3 |
| e2e-near-all-in-d25e00 | 8fac15d6bf2fb7b957eb6b86059a24dd70465c62d707e7f9f26b66253b7021d6 | 3f36802fc96f1ed6a288984ac73f8faea0ad5698f9d6b85eb3295f2937f64344 |
