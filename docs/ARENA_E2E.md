# Arena end-to-end run with the real SHARINGAN-1 blueprint

Date: 2026-10-28. Everything below was measured, not estimated. Raw per-run JSON stayed on the box in `~/dev-real` (`results-sdk.jsonl`, `results-house.jsonl`, `serve-latency-*.json`).

## Setup

| | |
|---|---|
| Blueprint | `C-medium-2e10.plcp`, the C@2e10 checkpoint that played live on Open Poker. 124,681,168,783 bytes, 10,516,669,987 infosets, compact PLCP1, mmapped |
| Abstraction | sizing `medium`, buckets `tables:data/buckets-200` (264 MB). Blueprint fingerprint reported by the arena: `f22c4ba8c43c0720` |
| Arena | `sharingan-arena` (profile `serve`), port 8100, `ARENA_BLUEPRINT=<above> ARENA_SIZING=medium ARENA_HAND_DELAY_MS=0`, SQLite |
| Inference API | `sharingan-serve`, port 8101, same blueprint, defaults (search on turn/river only, `flop=0`, 200k iterations, 5 s budget, cooperative cap 3.5 s), `--search-threads 8`, `--no-auth`, rate limit raised (see caveats) |
| SDK | `poker-sharingan` 0.1.0 (Python 3.12, websockets 17.3) |
| Box | 128 cores, 566 GB RAM, running the multi-day training job (load average about 130 throughout). All our processes ran at `nice 19`, pinned with `taskset` to at most 12 cores (arena or serve, never both at once) plus 2 for the Python client. Blueprint was fully in page cache (`MemAvailable` stayed above 255 GB). |

Units: **mbb/game** = thousandths of a big blind per hand (10 mbb = 1 bb/100). Score is the luck-corrected AIVAT estimate with its 95% CI half-width. Each run is 20,004 hands (20,000 rounded up to whole deals), `lanes: 4`, fixed track seed, so every bot sees the same deals.

## 1. Is `house:sharingan-1` the real bot?

Yes. `GET /v1/house` lists `house:sharingan-1` plus random, calling_station, tight_aggressive, nit, lag, maniac, chart6, chart6-loose, chart6-tight. `GET /v1/benchmarks` shows both tracks `available: true` with the fingerprint above. Two further checks that the menu and bucketer are right (a wrong menu gives garbage like -2364):

* 20k hands of `house:sharingan-1` against five `house:sharingan-1` on the sharingan-1 track: **-0.6 ± 4.8** mbb/game.
* chart6 against five SHARINGAN-1: **-135 ± 99**, in line with the earlier offline result for B-final vs 5x chart6 (+134 for the blueprint, docs/RESULTS.md).

Note that `house:sharingan-1` inside the arena is the **blueprint alone** (`CompactPolicy`). The live bot, and `sharingan-serve`, add turn/river search on top.

## 2. Benchmarks, 20,004 hands, lanes 4

Python SDK bots (remote, `verified: false`). `hands/s` is wall clock from run start to finished result, including the server-side AIVAT pass.

| bot | track | mbb/game ± 95% CI | hands/s |
|---|---|---|---|
| SDK `CallingStation` | ladder-v1 | -8572 ± 562 | 488 |
| SDK `CallingStation` | sharingan-1 | -6864 ± 473 | 521 |
| SDK `TightAggressive` | ladder-v1 | +4654 ± 469 | 1129 |
| SDK `TightAggressive` | sharingan-1 | -175 ± 117 | 1368 |

Server-side house bots (started with the admin token, `verified: true`, 3,500-5,500 hands/s):

| bot | ladder-v1 | sharingan-1 | hands/s (ladder / sharingan-1) |
|---|---|---|---|
| `house:tight_aggressive` | +6932 ± 414 | -353 ± 148 | 3876 / 4817 |
| `house:chart6` | +5288 ± 386 | -135 ± 99 | 3865 / 4284 |
| `house:calling_station` | -8440 ± 582 | -6879 ± 470 | 3890 / 4324 |
| `house:nit` | +3604 ± 317 | -113 ± 85 | 3884 / 5541 |
| `house:lag` | +8476 ± 563 | -1257 ± 268 | 3893 / 4862 |
| `house:maniac` | +5944 ± 663 | -2494 ± 472 | 3874 / 4850 |
| `house:random` | -6787 ± 508 | -4687 ± 453 | 3868 / 4852 |
| `house:chart6-loose` | +6535 ± 439 | -209 ± 114 | 3550 / 4322 |
| `house:chart6-tight` | +4130 ± 351 | -135 ± 89 | 3519 / 4886 |
| `house:sharingan-1` (control) | +2201 ± 287 | -0.6 ± 4.8 | 3870 / 4840 |

**Sanity: passed.** Every opponent loses to SHARINGAN-1 on the sharingan-1 track, and the CI excludes zero for all of them. The Python calling station (-6864 ± 473) and the server-side one (-6879 ± 470) agree, so the SDK path and the in-server path score the same. The SDK TAG and the server TAG are different implementations and score differently (-175 vs -353), as expected.

Hands/s depends on decisions per hand: the SDK calling station goes to showdown every hand (about 490-520 hands/s), the SDK TAG folds a lot (1100-1400). The README figure of about 4,800 hands/s for server-hosted bots reproduces (3,500-5,500 including about 1.8 s of finalize).

## 3. Inference API latency (`POST /v1/act`, search on turn/river, 8 search threads)

200 states per population, 50 per street, interleaved, one client, loopback, after a 12-request warm-up. States are produced by walking hands through the API itself.

* **self-play**: all six seats play SHARINGAN-1 (blueprint, sampled), so every history is one the bot could have produced. 87 of the 100 turn/river states are heads-up (the bot folds a lot).
* **random walk**: all seats play random legal actions with erratic sizes. Pots are multiway (82 of 100 turn/river states have 4+ players) and off the blueprint's menu.

Client-side latency in ms:

| street | source | self-play p50 / p95 / max | random-walk p50 / p95 / max |
|---|---|---|---|
| preflop | blueprint | 0.5 / 1.0 / 1.7 | 0.5 / 1.1 / 4.3 |
| flop | blueprint | 2.9 / 4.0 / 13.5 | 3.0 / 5.3 / 6.5 |
| turn | search | 420 / 1658 / 2113 | 1123 / 2711 / 3509 |
| river | search | 227 / 555 / 627 | 746 / 1629 / 1766 |
| **all 200** | | **79 / 583 / 2113** | **73 / 2213 / 3509** |
| turn + river only (100) | | 334 / 1095 / 2113 | 891 / 2485 / 3509 |

Server-reported `search_ms` matches the client numbers to within 5 ms. 0 fallbacks, 0 timeouts, 0 busy, 0 panics in either run; every turn/river answer was `source: "search"`. With two concurrent clients (the default `--search-concurrency 2`, 40 turn/river requests): self-play p50 394 / p95 2081 / max 3614 ms, random-walk p50 1303 / p95 4205 / max 4782 ms (still under the 5 s budget; the 3.5 s cooperative cap is per search, queueing adds to it).

Legality and sanity:

* 400 of 400 answers legal (0 illegal actions); probability vectors sum to 1 within 2e-6.
* Strength sanity on the returned distributions, facing a bet on turn/river (self-play): straight or better 0.0 fold mass (n=3); trips 0.01 (n=3); one pair 0.64 (n=15); high card 0.44 (n=3). No straight-or-better hand was ever folded in either population.
* Hand-crafted spots (heads-up, natural lines, 3 seeds each, search vs blueprint): river top two pair facing a bet never folds (raise 81%, call 19%); river QsJs missed air facing a bet folds 99.9%; river nut flush raises 100%; turn AhAd facing a bet raises 100% (blueprint alone only calls); turn Kd3h air folds 100%.
* Search does change the answer when it can: in self-play, 75 of 100 searched answers differ from the blueprint distribution (mean total variation 0.39 where they differ; the argmax action type agrees in 76%).

## 4. Play page

`site/` served statically on 8102, `?api=http://127.0.0.1:8100&room=sharingan`, driven by headless Chromium (Playwright from `~/dev-web`, read-only). In `fast` mode (150 ms between hands), 10 hands in 5.9 s, 35 decisions taken (a random call/fold/raise mix, so the -56 bb result means nothing). The table had the human in seat 0 and five `house:sharingan-1` in seats 1-5. 0 console errors, 0 page errors, 0 failed requests; no horizontal scroll at 375 px. Default pacing (no `fast`): 4 hands in 10.4 s, same result. Screenshots: `real-play-1280-turn.png`, `real-play-1280-end.png`, `real-play-375-turn.png`, `real-play-375-end.png`.

## Findings

Nothing in the arena, SDK or play page was broken. Things worth knowing:

1. **`source: "search"` does not guarantee the search result was used.** `SearchPlayer::dist` falls back silently to the blueprint distribution when the search table has no accumulated average for the hero's actual hand at the current node, and the response still says `source: "search"` after the full search time was spent. Diagnosed on an instrumented copy of the tree on the box (`~/dev-real-dbg`, repo untouched): `Solution::probs` finds the slot but `Table::average` returns false ("nothing accumulated"). This happens when the hero's earlier actions in the hand are ones the bot would never take (belief mass about zero, for example checking back a set on the flop). Effect: in the random-walk population only 9 of 100 searched answers differ from the blueprint (their `probs` are exactly the blueprint's k/255 values), against 75 of 100 in self-play. The README example response shows the same k/255 values under `source: "search"`. The instrumented self-play smoke run (including the hand-crafted spots) had 47 searches that produced a search distribution and 3 that fell back. For a caller replaying a human's or another bot's hand this means turn/river answers are blueprint answers that cost up to 3.5 s. Suggested follow-up: report `source: "blueprint"` (or a `fallback_reason`) when the search distribution is not available, and skip the search when the hero's reach is zero.
2. **Default serve rate limit (20 rps, burst 40 per key) blocks bulk callers.** My state generator got 429s immediately; the measurement runs used `--rate-rps 1000000 --rate-burst 1000000`.
3. **Off-menu histories give degenerate blueprint answers.** In the random-walk population the blueprint returns pure check/call (probability 1.0) in many states, including calling an all-in with 8s9s on K K 4 T J. This is the documented "unknown infoset means check/call" fallback, not a bug, but it makes the random-walk population a stress test, not a strength test.
4. **`python3 -m venv` does not work on the box** (no `ensurepip`, no `uv`). Used `python3 -m venv --without-pip` and installed the SDK with another venv's `pip --python`.

## Caveats

* The box was running the training job at about 126 cores while we measured (load about 130, our processes at `nice 19`), so hands/s and latency are pessimistic against an idle box. Search used 8 threads (the serve default is 16), so search latency would shrink on a dedicated machine.
* 50 states per street is a small sample: p95 and p99 are coarse. The 200 states are drawn from synthetic hands, not live ones. In self-play most turn/river states are heads-up, which is cheaper than multiway; the random-walk population is the pessimistic bound.
* The arena's `house:sharingan-1` is blueprint-only, so the sharingan-1 track measures opponents against the blueprint, not against the final search config.
* One 20k-hand run per bot and track (fixed track seed), so the CIs are the AIVAT CIs of a single run, not run-to-run variation.
* The SDK `TightAggressive` was run with its stock code; no bot code was modified.

## Reproduce

```bash
cargo build --profile serve -p sharingan-arena -p sharingan-serve
ARENA_BLUEPRINT=C-medium-2e10.plcp ARENA_SIZING=medium ARENA_BUCKETS=tables:data/buckets-200 ARENA_HAND_DELAY_MS=0 \
  ARENA_BIND=127.0.0.1:8100 ./target/serve/sharingan-arena
sharingan benchmark mybot.py --track ladder-v1 --hands 20000 --lanes 4 --server http://127.0.0.1:8100
curl -X POST $H/v1/benchmarks/sharingan-1/runs -H "authorization: Bearer $ADMIN" -H 'content-type: application/json' \
  -d '{"player_id":"house:chart6","hands":20000,"lanes":4}'
./target/serve/sharingan-serve --blueprint C-medium-2e10.plcp --buckets data/buckets-200 --sizing medium --search-threads 8 --no-auth
```
