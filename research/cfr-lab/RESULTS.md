# cfr-lab results

Leduc 2p (9,457 nodes, 936 infosets), exact exploitability = (BR1+BR2)/2 in chips
(x1000 = milli-chips). Reproduce: `cargo run --release -p cfr-lab --example curves [max]`.
Uniform-random profile: 2.374 chips (Kuhn 2p uniform: 0.458). Full-traversal solvers use
alternating updates; wall time for 5,000 iterations is under 1 s each.

Average-strategy exploitability (milli-chips):

| iters | CFR | CFR+ | LCFR | DCFR(1.5,0,2) |
|---:|---:|---:|---:|---:|
| 10 | 889 | 610 | 721 | 779 |
| 100 | 95.7 | 13.4 | 34.5 | 7.75 |
| 1,000 | 11.8 | 0.257 | 5.93 | 0.192 |
| 5,000 | 3.64 | 0.015 | 1.79 | 0.009 |

Current iterate (regret matching) is far worse for CFR/LCFR (~500-700 milli at 5k) and
noisy for CFR+/DCFR (2.2 / 7.6 milli at 5k): only the average carries the guarantee in 2p.

MCCFR-P (Pluribus Algorithm 1; schedule scaled to a 1M-iteration run: discount every 10
iterations until 400k, pruning from 200k with C=-5 chips in 95% of iterations, regret
floor -5.5 chips, i32 regrets at 1/1000 chip, strategy counter every iteration, seed 1):

| iters | avg expl (milli) | current-iterate expl (milli) |
|---:|---:|---:|
| 1,000 | 1156 | 1461 |
| 10,000 | 383 | 691 |
| 100,000 | 121 | 321 |
| 300,000 | 71.7 | 264 |
| 1,000,000 | 40.5 | 243 |

The average falls roughly as 1/sqrt(t) (sampling noise), as expected for external sampling.
1M iterations take 1.3 s. The current iterate stays ~0.25 chips exploitable: Pluribus
plays the final iterate only because search/abstraction make it robust; in raw Leduc the
average is the right thing to play.

Other games (tests): Kuhn 2p (12 infosets) CFR 20k: 7e-5, CFR+/LCFR/DCFR 5k: <5e-5, P1
value -0.05556 (= -1/18); MCCFR-P 200k: 2.8e-3. Kuhn 3p (48 infosets) NashConv uniform
2.06 -> CFR+ 3k iters ~0, MCCFR-P 300k 0.015. Leduc 3p (6 cards, 396k nodes, 13,878
infosets): NashConv uniform 12.54 -> CFR+ 100 iters 0.18.
