# search-lab results

Pluribus-style depth-limited nested unsafe search (supp. "Further details of the real-time
search algorithm", Alg. 2, "Leaf node values") on Leduc, scored with EXACT best response.
Reproduce: `cargo run --release -p search-lab --example report 2` (and `report 3 300`);
tests: `cargo test --release -p search-lab` (8 tests, ~2 s).

Naming: **R1** = first betting round (search root = start of game), **R2** = second round
(root = flop-like public state: round-1 history + board card). Leduc has 2 rounds, so R2
search always runs to the end of the game (no depth limit exists there); the depth limit /
continuation strategies only matter for **R1** search (leaves = start of R2).

## Design

* `blueprint`: (a) **MCCFR-P** average (cfr-lab Alg. 1, 3,000 it 2p / 20,000 it 3p, seed 7);
  (b) **card-abstract**: R2 infosets coarsened to {paired with board, unpaired high card,
  unpaired low card} (suits/exact ranks dropped, own R1 card forgotten = imperfect recall),
  solved with CFR+ in the abstract game (2000 it 2p / 300 it 3p), lifted by abstract-key
  lookup; (c) **equilibrium** CFR+ on the real game (5000 it 2p / 100 it 3p; 3p is not an
  exact equilibrium, NashConv 183); (d) **R1-flawed**: R1 from MCCFR-P 300 it spliced onto
  the equilibrium's R2 - a blueprint whose only flaw is early play, which isolates what
  depth-limited leaf values can repair.
* `beliefs`: joint reach of every tree node under a profile (one forward pass); normalising
  over a public state's nodes is Bayes with card removal (impossible deals have no node).
  Test: marginals equal an independent brute-force replay of the `Game` over all deals to
  1e-12 at 50+ public states, and the root is uniform.
* `subgame`: a fresh `cfr_lab::Tree` = chance root (probs ~ joint belief, zero-belief hands
  dropped = "unsafe") over the real betting tree of the round, sharing real infosets. Leaf
  (chance node of the next round): each player still in the hand gets a choice node with 4
  actions {blueprint, fold x5, call x5, raise x5} (renormalised at every later decision),
  infoset = (player, own card, public path) so choices are simultaneous; value of a joint
  choice = exact tree-walk EV of the rest of the game. Variants: `Cont4` (all choose),
  `Modicum` (only opponents choose, searcher = blueprint; one solve per searcher seat, each
  seat keeps its own solve), `Blueprint` (plain blueprint values). `Depth::Game` removes
  leaves. Solver: unmodified cfr-lab `Cfr` Linear, 2000 it (2p) / 300 it (3p).
* `composite`: per round in order, beliefs <- reach under the *belief* profile (blueprint,
  then previous search's AVERAGE), solve each root public state, write FINAL iterate (or
  average) to the played profile and the AVERAGE to the belief profile. Evaluate the played
  profile with `eval::exploitability` / `nash_conv`.

## 2-player exploitability (milli-chips; head-to-head EV vs the blueprint in parentheses)

Uniform play = 2374. Head-to-head = chips/hand x1000, averaged over both seats.

| search variant | MCCFR-P | card-abstract | equilibrium | R1-flawed |
|---|---:|---:|---:|---:|
| **blueprint alone** | **714.3** | **638.5** | **0.0** | **1113.9** |
| R2 only (blueprint R1), final | 327.2 (+186.2) | 673.5 (+86.6) | 420.3 (+0.0) | 978.4 (+98.3) |
| R2 only, average | 202.0 (+184.7) | 374.7 (+80.4) | 51.8 (-0.0) | 871.0 (+98.4) |
| R1 only cont4 depth-limited, final | 721.3 (+165.6) | 634.8 (-2.2) | 67.0 (+0.0) | 67.0 (+87.9) |
| R1 only cont4 depth-limited, average | 718.1 (+163.8) | 633.8 (-2.8) | 30.7 (+0.0) | 30.7 (+78.8) |
| R1 only blueprint-leaf depth-limited, final | 735.3 (+180.3) | 667.9 (+1.5) | 759.6 (-0.0) | 759.6 (+138.4) |
| R1 only blueprint-leaf depth-limited, average | 738.6 (+166.8) | 664.9 (+1.1) | 585.2 (-0.0) | 585.2 (+122.1) |
| R1 only Modicum depth-limited, final | 792.2 (+149.6) | 702.7 (-4.2) | 123.1 (+0.0) | 123.1 (+91.0) |
| R1 only Modicum depth-limited, average | 755.9 (+151.9) | 703.6 (-4.4) | 1.1 (-0.0) | 1.1 (+86.9) |
| R1 only full-depth (re-solve whole game), average | 3.3 (+229.3) | 3.3 (+26.0) | 3.3 (-0.1) | 3.3 (+86.2) |
| R1+R2 cont4 depth-limited, final | 505.8 (+303.6) | 1166.8 (+44.3) | 572.6 (-0.2) | 572.6 (-13.6) |
| R1+R2 cont4 depth-limited, average | 488.0 (+290.8) | 999.8 (+38.7) | 320.5 (-0.7) | 320.5 (-44.2) |
| R1+R2 blueprint-leaf, final | 676.1 (+290.4) | 1138.9 (+41.9) | 1070.0 (-100.4) | 1070.0 (+182.9) |
| R1+R2 blueprint-leaf, average | 694.0 (+265.6) | 1041.4 (+33.0) | 805.1 (-80.7) | 805.1 (+238.5) |
| R1+R2 Modicum, final | 692.4 (+212.8) | 764.0 (+90.2) | 716.5 (-0.0) | 716.5 (+0.8) |
| R1+R2 Modicum, average | 579.6 (+222.1) | 792.9 (+79.1) | 249.0 (-0.1) | 249.0 (+31.5) |
| R1+R2 full-depth, final | 418.7 (+201.7) | 418.7 (+0.8) | 418.7 (-0.0) | 418.7 (+22.8) |
| R1+R2 full-depth, average | 221.0 (+212.8) | 221.0 (+22.4) | 221.0 (-0.0) | 221.0 (+8.1) |

## 3-player NashConv (milli-chips; uniform = 12,540)

| search variant | MCCFR-P | card-abstract | equilibrium | R1-flawed |
|---|---:|---:|---:|---:|
| **blueprint alone** | **2754.1** | **2085.9** | **183.0** | **4942.6** |
| R2 only (blueprint R1), final | 1880.0 | 2497.4 | 1723.4 | 3527.5 |
| R2 only, average | 721.4 | 1576.1 | 134.2 | 3101.0 |
| R1 only cont4 depth-limited, final | 4613.5 | 3629.0 | 899.3 | 899.3 |
| R1 only cont4 depth-limited, average | 3374.6 | 3110.1 | 322.4 | 322.4 |
| R1 only blueprint-leaf depth-limited, final | 2999.4 | 3586.5 | 6410.1 | 6410.1 |
| R1 only blueprint-leaf depth-limited, average | 2871.6 | 2867.5 | 5438.0 | 5438.0 |
| R1 only Modicum depth-limited, final | 4026.9 | 3475.2 | 1785.2 | 1785.2 |
| R1 only Modicum depth-limited, average | 3929.0 | 3128.4 | 948.4 | 948.4 |
| R1 only full-depth (re-solve whole game), average | 279.5 | 279.5 | 279.5 | 279.5 |
| R1+R2 cont4 depth-limited, final | 3397.7 | 4272.2 | 2748.8 | 2748.8 |
| R1+R2 cont4 depth-limited, average | 1818.0 | 2189.5 | 674.1 | 674.1 |
| R1+R2 blueprint-leaf, final | 2338.2 | 5052.8 | 5090.2 | 5090.2 |
| R1+R2 blueprint-leaf, average | 1850.7 | 4014.3 | 3480.7 | 3480.7 |
| R1+R2 Modicum, final | 3626.2 | 4132.4 | 3412.2 | 3412.2 |
| R1+R2 Modicum, average | 2435.0 | 2669.4 | 1477.3 | 1477.3 |
| R1+R2 full-depth, final | 2394.7 | 2394.7 | 2394.7 | 2394.7 |
| R1+R2 full-depth, average | 175.8 | 175.8 | 175.8 | 175.8 |

(Test `three_player_nashconv_drops` uses 200 iterations: R2-only avg 728, R1+R2 cont4 avg
1884 vs blueprint 2754.)

## Findings

1. **R2 search (lossless re-solve of the last round from Bayes beliefs) is the big win.**
   MCCFR-P 714 -> 202 (average), card-abstract 639 -> 375, 3p NashConv 2754 -> 721; it also
   wins head-to-head (+185 / +80 milli). It repairs card abstraction because the subgame is
   lossless in the current round, exactly as in the supplementary.
2. **Depth-limited R1 search with 4 continuation strategies needs good leaf values to help.**
   With a sound R2 behind it (R1-flawed blueprint, 1114) cont4 gives 31 (average) / 67
   (final), vs plain blueprint leaf values 585 / 760, vs Modicum 1 / 123. On the MCCFR and
   abstract blueprints the R1 leaves inherit the blueprint's R2 flaws, so R1-only search is
   neutral (721 vs 714; 635 vs 639) - leaf quality caps what depth-limited search can fix.
   Ordering cont4 < blueprint-leaf holds in every R1-only 2p row; the plain-blueprint leaf
   is badly exploitable when the blueprint is an equilibrium (585) - the exact failure mode
   the supplement describes (opponent shifts continuation to exploit searcher).
3. **Cont4 vs Modicum**: Modicum was *better* in 2p exact exploitability here (1.1 vs 30.7)
   but worse in 3p (948 vs 322). The supplement's claim (Modicum is sound in 2p but gives
   the opponent more power; cont4 balances) is consistent with the 3p evidence, not with the
   2p exploitability number.
4. **Stacking R1 and R2 unsafe search hurts**: R1+R2 is worse than R2 alone for MCCFR
   (488 vs 202), abstract (1000 vs 375) and equilibrium (320 vs 52) blueprints. Unsafe
   re-solves of a later round against beliefs from a *different* (searched) R1 strategy are
   exploitable because an equilibrium R1 makes the opponent indifferent, so he can change
   his R2 range for free. Even R2-only from the exact equilibrium costs 52 (average) and
   grows with solver precision (47 at 500 it, 123 at 8000 it): classic unsafe-search
   exploitability, not a bug (game value is preserved: -0.0856 before and after).
5. **Full-depth R1 = re-solving the whole game**: 3.3 milli (2p, 2000 it), independent of the
   blueprint; only possible because Leduc is tiny.
6. **Final vs average**: in Leduc the average is better in nearly every row (e.g. 202 vs 327,
   31 vs 67); the current iterate of regret matching is jumpy in a tiny game. Pluribus'
   claim that the final iterate is unpredictable enough in NLHE cannot be tested here;
   beliefs always use the average, as in the paper.
7. 3p: R2-only average cuts NashConv 2754 -> 721 (MCCFR) and 2086 -> 1576 (abstract);
   R1+R2 cont4 average 1818 / 2190 (also below the blueprint for both); cont4 R1-only on the
   equilibrium-ish blueprint 322 vs 5438 for plain blueprint leaves.

## Implications for the NLHE implementation

* Always search post-flop rounds from the round start with beliefs from the blueprint
  (preflop: blueprint, as Pluribus does); that is where the exact gains are (finding 1).
* Use cont4 (not blueprint-only) leaf values when depth-limiting; build the continuation
  profiles lazily from the blueprint (fold/call/raise x5) and make the choice an infoset
  of (player, own hand bucket, public path) at leaves.
* Keep beliefs from the **average** strategy. For the played strategy, Leduc says average
  is safer; if using the final iterate (paper), use many iterations and measure with LBR
  before committing. Do not search R1 and R2 independently from fresh beliefs without
  checking exploitability (finding 4); AIVAT head-to-head alone could miss it.
* Iterations: 500-2000 Linear CFR it/subgame sufficed in Leduc; precision beyond ~500 did
  not reduce composite exploitability (unsafe error dominates).

## Deviations from the paper

* Leduc has no off-tree actions, so "freeze own actions" (Alg. 2 l.13) and re-search on
  off-tree actions are vacuous; each round root is searched once. All players' strategies
  in a subgame are re-solved and written (symmetric composite) for exact evaluation,
  instead of only the acting player's hand.
* Full traversal Linear CFR (alternating updates, cfr-lab) instead of MC Linear CFR;
  continuation strategies are not sampled-action compressed; no search-abstraction
  (lossless in all rounds except the card-abstract blueprint itself).
* 3p blueprint "equilibrium" is CFR+ 100 it (not an exact equilibrium).
* Zero-belief hands/public states keep blueprint play (unsafe search skips them).
