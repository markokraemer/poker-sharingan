# bench: baseline results

Generated with `bench matrix --deals 20000 --seed 7` (release, M4 Max; matrix ~2.5 min) and
`bench lbr`. Built-ins are untuned heuristics (see `src/builtin.rs`); numbers are for
orientation and regression, not strength claims. Each cell is the row policy against copies
of the column policy, mbb/game (milli big blinds per hand) with the AIVAT estimate and 95% CI.
Duplicate format: the hero plays every seat on the same deals (20,000 deals x 6 seatings in
6-max, x 2 in HU). AIVAT = full (chance nodes + all known-policy decisions, equity baseline).

### 6-max: row hero vs 5 copies of column (mbb/game, AIVAT, 95% CI)

| hero \ field | random | station | nit | tag | lag | maniac |
|---|---|---|---|---|---|---|
| random | - | -49.3 ± 102.9 | -6383.9 ± 189.7 | -12954.2 ± 251.4 | -14342.3 ± 242.9 | -8831.7 ± 155.9 |
| station | +4040.9 ± 327.6 | - | -4442.0 ± 138.4 | -14752.0 ± 263.2 | -17857.8 ± 315.3 | -6473.5 ± 190.9 |
| nit | +6476.1 ± 212.1 | +7614.6 ± 237.3 | - | +603.9 ± 53.5 | +2229.8 ± 104.9 | +3967.3 ± 117.4 |
| tag | +9977.7 ± 247.5 | +17069.4 ± 343.8 | -624.3 ± 55.8 | - | +2099.0 ± 117.1 | +4672.0 ± 127.1 |
| lag | +12919.2 ± 277.2 | +21459.3 ± 396.5 | -2479.7 ± 121.0 | -2766.0 ± 139.0 | - | +3520.0 ± 112.8 |
| maniac | +10943.9 ± 254.7 | +13490.4 ± 371.0 | -5091.6 ± 213.9 | -6455.9 ± 246.4 | -4570.2 ± 190.3 | - |

mean per-hand variance ratio raw/AIVAT: 4.5x (min 1.6, max 14.6)

### 2-max: row hero vs 1 copies of column (mbb/game, AIVAT, 95% CI)

| hero \ field | random | station | nit | tag | lag | maniac |
|---|---|---|---|---|---|---|
| random | - | -43.7 ± 23.8 | -1399.2 ± 97.3 | -3699.3 ± 151.4 | -6102.7 ± 193.1 | -7966.7 ± 243.2 |
| station | +43.7 ± 23.8 | - | -544.1 ± 30.1 | -1906.8 ± 57.2 | -3726.6 ± 91.7 | -8800.3 ± 218.4 |
| nit | +1399.2 ± 97.3 | +544.1 ± 30.1 | - | +91.9 ± 44.9 | +421.4 ± 82.2 | +822.0 ± 112.8 |
| tag | +3699.3 ± 151.4 | +1906.8 ± 57.2 | -91.9 ± 44.9 | - | +809.3 ± 111.2 | +1652.9 ± 153.1 |
| lag | +6102.7 ± 193.1 | +3726.6 ± 91.7 | -421.4 ± 82.2 | -809.3 ± 111.2 | - | +1653.8 ± 194.2 |
| maniac | +7966.7 ± 243.2 | +8800.3 ± 218.4 | -822.0 ± 112.8 | -1652.9 ± 153.1 | -1653.8 ± 194.2 | - |

mean per-hand variance ratio raw/AIVAT: 2.4x (min 1.6, max 5.4)

AIVAT raw/AIVAT variance ratios are per single hand (entrant payoff in one seating); cell-level
range shown above. The ratios are modest compared with the paper's ~9x because the baseline is
cheap check-down equity (no betting model) and the built-ins bet a lot; a better `ValueEstimator`
(e.g. policy rollouts or blueprint values) plugs in without changes elsewhere.
Unbiasedness is tested (`cargo test --release -p bench`): raw and AIVAT means agree within
error for HU and 6-max pairs; self-match is 0 within CI.

## LBR (lower bound on exploitability, mbb/game won by the LBR player)

Heads-up, `bench lbr --players 2 --deals 1500 --samples 300`, LBR in both seats, same deals:

| policy | LBR mbb/game (95% CI) |
|---|---|
| random | +9711 ± 1645 |
| station | +15477 ± 1221 |
| maniac | +2622 ± 1415 |
| lag | +1114 ± 1093 |
| nit | +668 ± 261 |
| tag | +578 ± 559 |
| toy blueprint (HU, 60k iters, coarse buckets, tiny sizing) | +2452 ± 1022 |

6-max profile LBR (one LBR seat vs 5 copies; NOT an equilibrium-gap bound),
`--players 6 --deals 300 --samples 150`: random +13531 ± 5092, station +20216 ± 4815,
maniac +3380 ± 3168, tag +1906 ± 1823, nit +1664 ± 820, lag +201 ± 2715.

LBR here is a weak exploiter (menu fold/call/0.5 pot/1 pot/all-in, check-down continuation
model, re-raises treated as calls), so every figure understates true exploitability. Tight
heuristics look "safe" because LBR cannot exploit their folding well, not because they are
close to equilibrium.

## Notes / limitations

- Blueprint checkpoints (`blueprint::io`) store only the `Store`; the bucketer and sizing are
  chosen by name at load (`--bucketer coarse|tables:<dir>`, `--sizing tiny|local|paper`).
- `bench slumbot` has not been run live.
