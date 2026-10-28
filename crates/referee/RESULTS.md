# referee results

All numbers produced on the dev box (`scripts/dev.sh`), seed 1, `RAYON_NUM_THREADS=16`.
Logs: `~/pluribus/logs/ref/*.json` on the box.

## Dataset

PokerBench (HF `RZ412/PokerBench`, license **Apache-2.0** per the HF card; the GitHub repo states none).
Downloaded the four `*_game_scenario_information.csv` files (the `prompt_and_label.json` files are
redundant prompt text) to `data/pokerbench/`; parsed as data only.

| file | rows |
|---|---|
| preflop test | 1,000 |
| preflop train | 60,000 |
| postflop test | 10,000 (635 flop / 4,097 turn / 5,268 river) |
| postflop train | 500,000 |

Postflop spots are heads-up (IP/OOP) after a 6-max preflop line; sizes are integer bb raise-to totals;
labels are one action (>50% solver frequency); test labels balance fold/check/call/bet-raise at 25% each.
**Coverage: 100% of all 574,200 rows replay** on the nlhe engine (test and train; 0 parse failures).
Pot cross-check against the source `pot_size`: ~94% within 1bb+3%; the rest are river spots where
integer-rounded source sizes accumulate (up to ~5bb).

Dataset quirks handled: `Fold` listed beside `Check`; `Call` omitted at some pruned nodes; folds of
skipped seats are implicit; preflop lines for HU postflop spots end before the remaining seats fold.

## PokerBench baselines (exact = argmax option equals label; mass = probability on label)

Test split (11,000 spots, coverage 1.0):

| policy | exact | class | mass_exact | mass_class | preflop | flop | turn | river |
|---|---|---|---|---|---|---|---|---|
| nit | 0.645 | 0.653 | 0.637 | 0.644 | 0.698 | 0.444 | 0.596 | 0.696 |
| tag | 0.512 | 0.522 | 0.510 | 0.520 | 0.608 | 0.362 | 0.471 | 0.543 |
| station | 0.500 | 0.500 | 0.500 | 0.500 | 0.500 | 0.624 | 0.493 | 0.491 |
| random | 0.500 | 0.500 | 0.446 | 0.452 | 0.500 | 0.624 | 0.493 | 0.491 |
| toy blueprint (60k it, coarse buckets, tiny sizing) | 0.450 | 0.449 | 0.453 | 0.458 | 0.488 | 0.487 | 0.438 | 0.447 |

Train sample (50k strided rows) agrees within ~0.01 for every bot (e.g. tag 0.514, nit 0.647).
Chance level is not 25%: check/call labels are 50% of the test set, so a station scores 0.50 exact.
Treat accuracy as regression only: a deterministic modal-action lookup scores high (nit wins here).
Tag breakdown highlights: label class bet_raise 0.63, fold 0.56, check_call 0.43; preflop vs_4bet shove nodes 0.31;
heads-up-postflop facing-bet-no-raise spots are easiest (0.69-0.75).

## Exact heads-up river referee (100 deduplicated river nodes from the test split, 1000 CFR+ iterations)

Ranges: Bayes from the policy's own action probabilities along the history (floor 1e-4). Menu: first bet
{0.5, 1.0 pot, all-in}, raises {1.0, all-in}, 3 raises. Mean pot 27.8bb. EV loss = `value_p(eq) + BR_opponent(policy_p)`,
mbb of the big blind, per spot, mean over both seats.

| policy | EV loss mbb (mean) | median | % of pot | calling-station control |
|---|---|---|---|---|
| tag | 11,095 | 9,239 | 56 | 27,959 |
| nit | 12,499 | 11,681 | 55 | 28,848 |
| random | 19,251 | 19,719 | 125 | 22,145 |
| station | 22,145 | 22,725 | 145 | 22,145 |
| toy blueprint | 29,538 | 28,351 | 185 | 31,229 |

Reference solver exploitability (mean) 35-58 mbb, i.e. under 0.5% of the loss, so the numbers are not solver noise.
(The control differs by policy only because ranges differ.) A strong blueprint should land far below tag; those numbers
are large because heuristics play river spots far from equilibrium in ranges they shape themselves.

## Solver validation (tests, `cargo test --release -p referee`, 7 pass in ~2s on the box)

* Showdown/fold evaluation matches an O(n^2) brute force to 1e-9 with card removal.
* Toy river (board TsTcThTd2c, OOP = AA + 33 half/half, IP = KK, pot-size bet, no raises): CFR+ 10k iterations
  gives values OOP +50 / IP -50 chips (analytic 0.25 pot), value hands bet 100%, air bluffs 50% (+-3%), IP calls 50% (+-3%),
  exploitability < 5 mbb.
* Equilibrium vs itself: loss < 10 mbb; a 300-iteration strategy loses < 100 mbb; a calling station loses > 100 mbb in both seats.
* Real spots: zero-sum check, solver exploitability < 5% pot at 400 iterations.
* Parser roundtrips every test-set preflop/postflop line; all 11,000 test spots replay with hero to act and legal options.

## Running against the real blueprint

```
DEV_TARGET=referee scripts/dev.sh run 'cargo build --release -p referee'
B=target/referee/release/referee
$B pokerbench  --policy blueprint:data/run-paper/export.bp --sizing paper --buckets data/buckets-200 --split test
$B pokerbench  --policy blueprint:... --sizing paper --buckets data/buckets-200 --split train --limit 50000
$B river-spots --policy blueprint:... --sizing paper --buckets data/buckets-200 --n 200 --iters 1500
```
Set `--capacity` (default 2^26 slots) to the checkpoint's size. `--ranges uniform` isolates strategy quality from range quality.

## Deviations / caveats

* `pluribus` crate not used as a dependency (bench provides `sizing_by_name`/bucketers); spec `blueprint:<path>` uses bench's loader; added `toy:<iters>`.
* Size mapping: raises map to the solver option with nearest raise-increment (log ratio); a raise when no solver raise exists is "unmapped" (never exact).
* Postflop spot sizes are integer-rounded; replay snaps sizes within 1bb of min-raise/all-in.
* River EV loss is range-conditional on the policy's own Bayes ranges, assumes the policy plays both seats, uses a fixed menu; turn spots not implemented.
* CFR+ here converges ~O(1/T) (70 mbb at 1000 its, 30 at 2000 on typical spots); use `ref_expl` in the JSON as the error bar.
* `river-spots --split train` loads the full train file; use the test split.
