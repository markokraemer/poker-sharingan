# Method notes

A compact description of what is implemented and where, following Brown & Sandholm (2019) and its supplementary material. The paper text is not redistributed here.

## 1. Game and engine (`engine/nlhe`)

* 2-6 player no-limit hold'em state machine with `to`-amount semantics: min-raise = current bet + last full raise (at least the big blind), a short all-in does not reopen betting,
  side pots, odd chip to the left of the button.
* The blueprint is trained at 100 bb. Search and evaluation use the same engine, so abstract and real states can be compared exactly.

## 2. Abstractions (`sharingan-1/abstraction`, `engine/iso`)

* **Cards:** preflop is lossless (169 strategic classes). Flop, turn and river use 200 buckets each (imperfect recall). Flop and turn: k-means with earth mover's distance over equity
  histograms. River: OCHS-style features. Hand indexing uses Waugh's suit-isomorphism algorithm (`iso`).
* **Actions:** fold/call always; a menu of raise sizes as pot fractions per street. The paper does not publish its exact preflop/flop menus, so three menus were built
  (`paper`-style, `local`, `medium`); `medium` won by measurement.
* **Search abstraction:** an optional per-flop 500-bucket abstraction exists (`search_buckets`) but is off in the final configuration because it did not help.

## 3. Blueprint (`sharingan-1/blueprint`, `sharingan-1/pluribus`)

* Parallel **external-sampling Monte Carlo CFR** with **Linear CFR** discounting `d = (t/10)/(t/10 + 1)` during the first part of the run.
* **Negative-regret pruning:** after a warm-up, in 95% of iterations actions whose regret is below a threshold (-300M) are skipped; never on the river or on actions leading straight to a terminal.
* **Averaging:** the preflop strategy is the running average; postflop uses the average of current-strategy snapshots (`snapshots.sum`).
* **Schedule:** the paper's wall-clock schedule is mapped proportionally to iterations (`scaled_schedule`): discount 0-3.5%, pruning from 1.7%, averaging from 6.9%.
* **Storage:** lock-free hash store with 8-byte history keys and a per-infoset index, lazy allocation, checkpoints PLBP4, compact policy PLCP1 (8-bit probabilities).

## 4. Action translation (`engine/nlhe`)

Opponent bets that are not in our menu are mapped to the abstraction. Naive nearest-size translation is exploitable at the 10^4 mbb/game level in 6-max. We track an abstract "shadow game" state
separately from the real game so subsequent decisions stay consistent with what the blueprint believes happened, which cut exploitability by ~37x (see RESULTS.md).

## 5. Real-time search (`sharingan-1/search`)

* Entered at the start of the current betting round (flop and later; preflop search is not used). Our own earlier actions are frozen for our actual hand.
* **Subgame solver:** nested unsafe depth-limited Monte Carlo Linear CFR, run on a thread pool; an exact vector CFR+ solver exists for heads-up spots and was found not to help in 6-max.
* **Leaves:** each remaining player picks one of **four continuation strategies** (blueprint, and fold-, call- and raise-biased versions), chosen per infoset.
* **Beliefs:** Bayesian updates over the 1326 hole-card pairs per player. For opponents we use the blueprint likelihood (they *are* the blueprint in our tests); our own seat uses the search average.
* **Off-tree actions:** the action is added to the subgame tree and the search is re-run from the root.
* **Final config:** 200k iterations, turn and river only (`--set flop=0`), hard time budget with blueprint fallback in live play.

## 6. Evaluation

See [EVALUATION.md](EVALUATION.md).

## 7. What differs from the paper

See the "Deviations from the paper" section of the README.
