# Evaluation methodology

Poker results are extremely noisy: one big blind per hand is a typical standard deviation of several big blinds, and the edges that matter are
a few hundredths of a big blind per hand. Everything in [RESULTS.md](RESULTS.md) is therefore measured with variance reduction and reported with confidence intervals.
Units: **mbb/game** = thousandths of a big blind per hand (1 mbb/game = 0.1 bb/100). Positive = the first-named bot wins.

## Duplicate deals

In a 6-max duplicate match every deal (hole cards and board) is replayed with the hero rotated through **every seat** (and, for multi-bot formats, with seat permutations),
so luck of the draw and positional advantage cancel. The same seed gives the same deals across arms, which makes *paired* comparisons between two
configurations much tighter than independent runs. Formats used:

* `3A + 3B`: three copies of each bot, all seat assignments.
* `1A vs 5B` and `1B vs 5A`: one bot against five copies of the other, in every seat.
* `hero vs 5 x field`: one bot against five copies of a field bot (blueprint, rule bot, RLCard, SvanBot).

Resolution: about +-1-2 mbb/game needs ~4M deals per format; a 7k-deal search run is only +-70.

## AIVAT

`crates/bench/src/aivat.rs` implements AIVAT (Burch et al. 2018) in the multi-player form of the Pluribus supplement. For each hand the realised payoff is corrected by
zero-mean terms at chance nodes (hole-card deals, board reveals including all-in run-outs) and at decision nodes of bots that expose their action probabilities.
The estimate is unbiased for any baseline value function; a good baseline only lowers variance. The default baseline assumes every live player calls the outstanding bet and the hand is
checked down (exact enumeration postflop, sampling preflop). `Mode::CardsOnly` is the paper's simpler variant. Decisions of non-exposing players (external bots) get no correction.
Run with `--aivat full|cards|off` in `bench`.

## Exploitability proxies

* **Trained best response (`crates/exploit`)**: a learner (an ABR learner; 12M iterations in our runs) is trained against a *frozen* compact blueprint and then played against five copies of it in a duplicate match,
  rotated through every seat. A negative result means the exploiter loses to the target. It can use the target's own action menu (`same`) or a different menu translated into the target's (`cross-menu`, e.g. the paper menu).
  It is bounded by its own abstraction and iteration count, so it gives a **lower bound** on exploitability, not a guarantee. Command: `pluribus exploit-compact` (see the README).
* **LBR** (`bench lbr`): local best response, the cheaper classic estimate.
* Toy-game verification (`crates/cfr-lab`, `crates/search-lab`): exact exploitability on Kuhn/Leduc to check the solvers.
* `crates/referee`: spot-by-spot checks (PokerBench accuracy; exact heads-up river CFR+ EV loss).

## Rule bots and external bots

* `chart6`, `chart6-loose`, `chart6-tight`, `tag`, `lag`, `nit`, `station`, `maniac` (`bench::builtin`, `bench::chart`): deterministic rule-based opponents, so AIVAT can treat them as known strategies.
* RLCard NLHE DQN (`scripts/extbots/`): a Python bridge speaking JSON lines (`ext:` policies, protocol in `crates/bench/src/ext.rs`). RLCard has no pretrained NLHE model, so a DQN is trained first (`rlcard_train.py`).
* Slumbot (heads-up, `bench slumbot`): live API client; 2,400 hands are far too few to resolve anything small.
* SvanBot: an external bridge (`scripts/extbots/svan/`) that needs SvanBot's source code; it is not redistributed (see NOTICE).

## Live play (Open Poker)

`crates/openpoker` plays on the public Open Poker bot arena, traces every message, and its analyzer converts traces to bb/100 with a confidence interval and an
**all-in-luck adjustment** (all-in pots are re-valued at their equity). Thousands of hands are needed for a CI that excludes zero; ~10^5 for +-5 bb/100.
Run `openpoker analyze --trace <file>`.

## How to benchmark your own agent

1. Implement the policy API in `crates/bench/src/policy.rs`, or write a JSON-lines bridge (`ext:` protocol) in any language.
2. Seat it as hero against five copies of a field bot with `bench table --hero <yours> --field <field> --deals N --seed S` and read the AIVAT line (use `--aivat off` if your bot does not expose probabilities and compare raw duplicate results).
3. Use the same `--seed` for every arm you want to compare, and report the CI.
4. For an open, hosted version of this (public play-against API and benchmark) see "Coming soon" in the README.

## Pitfalls we hit

* `eval_checkpoint.sh` reusing a `--tag` once picked up **another run's sizing** from the default run dir. A medium
  compact evaluated with the paper menu gives −2364 mbb/g garbage. Fix: the tag dir stores `sizing`. Still always pass
  `--run-dir`/`--sizing`.
* The gauntlet's "2-max" section seats a 6-max blueprint heads-up. Ignore it.
* Same-seed paired comparisons and 4M-deal h2h are needed for ±1–2 mbb/g resolution. 7k-deal search runs are ±70.


## Live-play pitfalls

* Cash-game stacks **accumulate**: the bot reached 1,105 bb and its 100 bb-trained "all-in" became a 1,105 bb AK jam.
  Fix: `--max-stack-bb 150`, which does leave-and-rejoin at the buy-in plus a per-hand commit cap. Verified live (25 resets).
  Pluribus avoided this by resetting stacks every hand.
* Other server behaviour:
  * `cannot_rebuy` while the season balance > 0 → rejoin the lobby;
  * reconnect give-ups happen → run under a restart loop;
  * the release profile has `panic=abort`, so a panic kills the bot → the restart loop is mandatory.
* A few hundred live hands are noise (±150 bb/100). You need thousands for a CI that excludes 0, and ~10⁵ for ±5 bb/100.
