# External-bot evaluation: how does blueprint B do against opponents we did not build?

Researched and run 2026-10-13. Everything computational ran on the dev box (`scripts/dev.sh`); third-party
code lives in `~/extbots/<name>/` there (own directory per package, run with `python3 -I` / the venv's
`python -I`). Nothing was executed on the Mac. Local evaluation only; no third-party code is redistributed.
Tags: **[V]** = read on the primary page today, **[S]** = search snippet / secondary.

## 1. Research: what strong NLHE bots can one actually run or play today?

Headline: **there is no strong, openly runnable, pretrained 6-max NLHE agent anywhere** (learned or
rule-based). Pluribus, Libratus, DeepStack and ReBeL weights/poker code were never released; every
open-source "Pluribus" is a trainer you must run for days with your own abstraction. The only strong
opponent one can *play* is heads-up (Slumbot, and GTO Wizard's key-gated API). So external 6-max
assurance has to come from (a) a disciplined rule-based bot, (b) the one learned NLHE agent the public
toolkits can produce (RLCard), (c) our own diverse bot field, and (d) the HU Slumbot check for the
shared machinery (blueprint quality, translation, play code).

| Name | Format | Type / strength evidence | License | Runnable today? | Integration effort | Decision |
|---|---|---|---|---|---|---|
| **Slumbot** (slumbot.com; [GTO Wizard Benchmark paper](https://arxiv.org/abs/2603.23660)) | HU, 200 bb, 50/100 | Fixed CFR strategy, ACPC 2018 HU winner; GTO Wizard AI beats it by 19.4 +/- 4.1 bb/100 [V] | public free service | **Yes, HTTP API live today** (`new_hand`/`act`, verified) | client already in `bench::slumbot` | **Played live (section 4)** |
| **GTO Wizard Benchmark API** ([paper](https://arxiv.org/abs/2603.23660)) | HU 200 bb | Strongest public HU bot (proprietary) | key by manual request | **No key** | adapt the Slumbot grammar | Not run (no key) |
| [slumbot2019 source](https://github.com/ericgjackson/slumbot2019) | HU (MCCFR also multiplayer, partial) | Solver toolkit; no strategy files or playable bot shipped [V] | MIT | Build yes, but you must solve it yourself (100M-iteration MCCFR examples + card abstraction) | days of compute | Skipped (would be our own trainer again) |
| [DecisionHoldem](https://arxiv.org/abs/2201.11580) / [OpenI repo](https://openi.pcl.ac.cn/chenhao/DecisionHoldem) | HU | Paper: beats Slumbot by >730 mbb/h (~73 bb/100, methodology unverified) [V abstract] | AGPL-3.0 | Source only; blueprint file is on a Baidu drive or 3-4 days on 48 cores to rebuild; some code only as binaries [V] | high; AGPL | Skipped (no obtainable weights; HU only) |
| [DeepHoldem](https://github.com/happypepper/DeepHoldem) | HU (DeepStack-style) | Author claims +42 bb/100 vs Slumbot 2017 over ~2.6k hands (noisy) [V] | none stated | Lua/Torch; only the preflop net ships, flop/turn/river nets must be generated (GPU, long) | very high | Skipped |
| [fedden/poker_ai](https://github.com/fedden/poker_ai) | "6-max", 20-card short deck | MCCFR blueprint trainer, no strength claim; archived 2024 [V] | GPL-3.0 | needs its own clustering + training | n/a | Skipped (not a strong bot) |
| [ozzi7/Poker-MCCFRM](https://github.com/ozzi7/Poker-MCCFRM) | any N players | Loose Pluribus re-implementation, no results; no releases; Windows/VS build; >128 GB RAM advised [V] | MIT | Build-it-yourself | n/a | Skipped |
| [PokerRL](https://github.com/EricSteinberger/PokerRL) (Deep CFR, SD-CFR) | small NL poker, N players in engine | Research framework, no pretrained agents [V] | MIT | Framework only | n/a | Skipped |
| [OpenSpiel](https://github.com/deepmind/open_spiel/blob/master/docs/games.md) `universal_poker` | ACPC-style NLHE expressible | Game engine; no pretrained NLHE agents [V] | Apache-2.0 | Engine only | n/a | Skipped |
| [ReBeL](https://github.com/facebookresearch/rebel) | Liar's Dice only in the repo | Poker agent never released [V] | Apache-2.0 | no poker | n/a | n/a |
| Pluribus / Libratus / DeepStack | 6-max / HU | Superhuman; code and weights withheld ([Pluribus](https://en.wikipedia.org/wiki/Pluribus_(poker_bot))) | - | **No** | - | n/a |
| [OpenHoldem](https://documentation.help/OpenHoldem/) (GPL-3.0) + profiles | any table via screen scraping | Framework with a default "Gecko" profile; strong commercial profiles ("Shanky" style) are paid, closed products | GPL-3.0 | Needs a poker client to scrape | high, and profiles unobtainable legally | Replaced by our own chart bot (below) |
| **RLCard** NLHE ([RLCard](https://pypi.org/project/rlcard)) DQN / NFSP / DMC | NLHE, N players, 5 abstract actions | Model zoo has **no** NLHE model (only Leduc, limit rule, UNO, Dou Dizhu, gin) [S]; but the toolkit trains a DQN you can run | MIT | **Yes** (pip, CPU) | JSON-lines bridge + train script (done) | **Integrated: `rlcard:` (learned)** |
| **chart-based rule bot** (published-chart shape: RFI charts + 3/4-bet bands + equity/pot-odds postflop) | 6-max, 100 bb | Not a downloaded product: a native Rust re-implementation of the standard "chart + pot odds" regime human regs and rule bots use | ours | Yes | done (`chart6`, `-loose`, `-tight`) | **Integrated: `chart6*` (deterministic rule)** |
| Open Poker (openpoker.ai) | 6-max, live bots | Other developers' bots; already running (separate live bot) | free | Yes | existing adapter | Not part of this study |

Notes on strength evidence: Slumbot is a fixed non-adaptive strategy, hard to beat head-to-head yet
highly exploitable in the LBR sense; ~2018 ACPC level. "GTO Wizard AI beat it by ~19 bb/100" is the
yardstick for the HU play in section 4. Nothing external certifies 6-max strength; claims in this
document are therefore "beats every opponent we could obtain or build" and no more.

## 2. What was integrated

All opponents are bench `PolicyFactory` specs usable in `bench match|table|gauntlet|matrix`:

| spec | what | where |
|---|---|---|
| `chart6`, `chart6-loose`, `chart6-tight` | Deterministic rule bots. Preflop: position RFI charts (UTG ~16 %, HJ ~21 %, CO ~27 %, BTN ~44 %, SB ~42 % raise-or-fold), 3-bet/call bands by hero x opener position, 4-bet/5-bet-jam bands. Postflop: Monte-Carlo equity vs a range proxy (top fraction of villain's hands by expected future strength; narrower vs bigger bets and raises), pot-odds calls, value bets/raises, c-bets, hash-seeded bluffs. `-loose` (x1.45) / `-tight` (x0.7) scale widths and aggression. Pure function of (history, hole): `exposes_probs` is true, so AIVAT also corrects *their* decisions. The charts are approximations written from memory of commonly published ranges, **not** solver output. | `crates/bench/src/chart.rs` |
| `rlcard:<ckpt>` | Learned agent: stock RLCard DQN (256x256 MLP, 5 abstract actions: fold, check/call, 1/2 pot, pot, all-in) self-play-trained on 6-max 100 bb NLHE (RLCard ships no NLHE weights). Runs in a Python worker pool behind the bridge. | `scripts/extbots/rlcard_train.py`, `rlcard_bot_server.py` |
| `ext:<name>=<json argv>` | Generic bridge for any other bot: JSON lines over stdin/stdout, stateless protocol (full public state + history in every request), worker pool shared by all rayon threads, per-request timeout, first-request startup grace, kill+respawn of hung/dead workers, legalization of every answer (raise clamped to legal range, impossible raise becomes call/check, fold of a free check becomes check, malformed/timeout becomes check-or-fold and is counted in the end-of-run `report`). | `crates/bench/src/ext.rs` (protocol documented at the top) |
| `bench table --hero SPEC --field a,b,c,d,e` | Mixed tables (or one field spec = 5 copies); every cyclic rotation of the seating on every deal (duplicate), CRN, AIVAT. | `crates/bench/src/main.rs` |
| `bench slumbot` | Live HU client with `--pace-ms` (one request in flight, spaced), retry/backoff, per-hand JSONL log, and an all-in-EV "luck adjusted" estimate (Slumbot is not a known strategy so true AIVAT is impossible; every hand that reached showdown all-in before the river is replaced by its exact EV). | `crates/bench/src/slumbot.rs` |
| `pluribus train --stack N` | New flag (default 10000 = 100 bb) so a 200 bb HU blueprint can be trained. | `crates/pluribus/src/main.rs` |

Unit tests (`cargo test -p bench --lib -- chart:: ext:: slumbot::`, 10 pass): chart widths in the expected
bands and AA/72o ordering; chart6 legal, zero-sum and independent of the random stream; bridge request
contains the full public state; responses legalized (clamp, allin, check-vs-call, free-check fold, wrong id,
garbage, unknown action); a real Python subprocess plays full 6-handed hands with 0 fallbacks; a hung bot
(timeout) and a crashing bot (process exit) both fall back safely and are counted; all-in-EV maths
(AA v KK preflop ~ +12.8k chips).

## 3. 6-max results: blueprint B-final vs the external field

B = `data/eval/B-it20000000000-20261013T1354Z/compact.plcp` (2e10 iterations, snapshot-averaged, 2.38 B
infosets), `--sizing local`, buckets `data/buckets-200`, translation = default (deterministic
pseudo-harmonic, shadow state). Protocol: 100 bb, 50/100 blinds, **duplicate deals** (every cyclic
rotation of the seating on every deal, common random numbers), **AIVAT** (full mode; the rule bots and
built-ins expose their probabilities so their decisions are corrected too; RLCard is a sampling-free
black box, so only chance and B's own actions are corrected for it). All numbers: mbb/game for the entrant
(mean over the seats it occupies), AIVAT estimate with **95 % CI**. 100,000 deals = 600,000 hands per row
(20 s - 2 min on 16 threads). Raw (non-AIVAT) values and per-seat splits are in
`~/pluribus/data/extbots-results/*.txt` on the dev box. Seeds are fixed per row (see
`scripts/extbots/run_rules.sh`, `run_rlcard.sh`).

### 3a. Hero B vs 5 copies of one bot (B's number; the bot's number is -1/5 of it)

| opponent | B mbb/game (AIVAT) | per-opponent-seat |
|---|---|---|
| chart6 (rule, standard charts) | **+134.4 +/- 17.5** | -26.9 +/- 3.5 |
| chart6-loose (rule, LAG-ish) | **+173.5 +/- 19.5** | -34.7 +/- 3.9 |
| chart6-tight (rule, nit-ish) | **+129.5 +/- 16.4** | -25.9 +/- 3.3 |
| rlcard DQN (learned) | **+587.0 +/- 40.9** | -117.4 +/- 8.2 |
| nit (built-in) | +138.3 +/- 15.0 | -27.7 +/- 3.0 |
| tag (built-in) | +366.4 +/- 24.6 | -73.3 +/- 4.9 |
| lag (built-in) | +888.9 +/- 37.8 | -177.8 +/- 7.6 |
| maniac (built-in) | +1498.8 +/- 44.4 | -299.8 +/- 8.9 |
| station (built-in) | +3511.7 +/- 67.4 | -702.3 +/- 13.5 |

B is positive against every opponent, all CIs exclude 0 by 7 sigma or more, and every row reaches the
requested +/- 50 mbb/game (tightest +/- 15, widest +/- 67). B wins in every seat except the blinds, as
expected (e.g. vs chart6: SB -74, BB -265, UTG..BTN +187/+211/+282/+489 raw).

### 3b. The inverse view: ONE external bot at a table of 5 B's (the paper's "1 human + 5 AI" setup)

| lone opponent | its mbb/game (AIVAT) |
|---|---|
| chart6 | -111.9 +/- 17.0 |
| chart6-loose | -177.1 +/- 20.6 |
| chart6-tight | -98.9 +/- 14.9 |
| tag | -416.8 +/- 26.1 |
| lag | -1,308.3 +/- 48.3 |
| maniac | -2,397.5 +/- 81.5 |
| rlcard DQN | -1,778.8 +/- 48.3 |

None of them can beat five B's; the best of them (chart6-tight) still loses ~100 mbb/game. (For reference
the exploiter trained against B with B's own bet menu lost -205 +/- 57; none of these opponents is an
adaptive best-response, so this is a lower bound on exploitability, not a measurement of it.)

### 3c. Mixed tables (1 B + 5 *different* bots), 100,000 deals, all rotations

| table | B | chart6 | chart6-loose | chart6-tight | tag | lag | rlcard | nit | maniac | station |
|---|---|---|---|---|---|---|---|---|---|---|
| T1: B + chart6, loose, tight, tag, lag | **+430.1 +/- 25.0** | +583.1 +/- 28.3 | +571.1 +/- 31.5 | +514.0 +/- 25.5 | +241.5 +/- 36.1 | -2339.8 +/- 55.1 | | | | |
| T2: B + chart6, loose, nit, maniac, station | **+1864.5 +/- 42.8** | +3362.8 +/- 57.7 | +3910.7 +/- 65.5 | | | | | +2779.9 +/- 50.9 | +4064.3 +/- 111.2 | -15982.2 +/- 120.4 |
| T3: B + chart6, loose, tight, rlcard, tag | **+446.0 +/- 24.9** | +780.8 +/- 29.3 | +741.5 +/- 32.6 | +618.0 +/- 25.9 | +386.3 +/- 38.9 | | -2972.5 +/- 57.8 | | | |

Read these carefully: every table contains a donor (lag, station, rlcard) and the zero-sum identity makes
all the other seats positive. B is positive in all three, **but the chart rule bots earn more than B at
the same table** (e.g. T3: chart6 +781 vs B +446). This is not a head-to-head loss (3a/3b show B beats and
is not beaten by those bots); it says B is a fixed, non-adapting strategy that takes less from a leaking
opponent than a looser, more aggressive bot does (chart6 is 3-bet/c-bet heavy against calling stations).
So B is "unbeatable by these bots", not "the best exploiter at a mixed table". That is expected of an
equilibrium-style blueprint; exploiting weak tables is the job of the opponent-modelling layer.

### 3d. Context: how good are the external bots themselves?

| matchup (hero vs 5x) | hero mbb/game (AIVAT) | deals |
|---|---|---|
| chart6 vs tag / lag / nit / maniac / station | +603 / +1,723 / +169 / +2,137 / +15,796 | 3,000 |
| chart6 vs rlcard | +1,984 +/- 199 | 5,000 |
| rlcard vs random / station | +9,696 +/- 696 / +14,638 +/- 1,045 | 3,000 |
| rlcard vs tag / lag | -2,838 +/- 398 / +240 +/- 422 | 3,000 |

So chart6 is a genuinely competent rule bot (beats every built-in and the learned agent by a wide margin,
beats B's own gauntlet numbers on weak fields). The RLCard DQN (~380k self-play hands, 90 min, 4 threads; it
only sees cards plus two chip counts, no betting history) is a weak learned agent: it crushes random and
stations but loses to a TAG. It is the best *learned* NLHE agent obtainable off the shelf, which says
more about the public landscape than about the agent.

## 4. Heads-up vs Slumbot (public API, 200 bb, blinds 50/100)

* **Blueprint:** `pluribus train --players 2 --stack 20000 --sizing local --iters 700000000 --mem-gb 60`
  (new `--stack` flag; 32 threads, ~80 min on the loaded shared box, 6.26 M infosets, buckets-200)
  then `pluribus compact` (76 MB). The run dir is `data/run-hu200`, the policy `data/hu200/compact.plcp`.
  Blueprint only, no real-time search, Slumbot's off-tree bet sizes handled by the same translation as in
  6-max play.
* **Match:** 2,400 live hands (`bench slumbot --hands 2400 --pace-ms 400`; one request in flight at a time,
  >= 0.4 s apart, ~1.5 s per hand, 60 min total, ~12k requests; the log of every hand is
  `data/extbots-results/slumbot-hu200.jsonl` on the dev box). Slumbot's API reported no errors.
* **Result:** **+13.9 bb/100, 95 % CI +/- 68.7** raw; luck-adjusted (exact all-in EV for every hand that went
  all-in before the river) **+14.1 +/- 65.5 bb/100**. By seat: BB -24.9 +/- 91.3, SB/button +52.8 +/- 102.5.
  Hand standard deviation ~16 bb, so 2,400 hands cannot resolve anything under ~65 bb/100.

Honest reading: the blueprint is **not losing** to Slumbot (point estimate positive, CI [-55, +83]) but
this match does not demonstrate that it *beats* it. Slumbot (ACPC 2018 champion level) is beaten by GTO
Wizard AI by 19.4 +/- 4.1 bb/100 (150k hands, AIVAT); to detect a ~15 bb/100 edge at 95 % one needs
~40-50k hands (~15-20 h at the polite pace used here), so we stopped at the requested 2,000+ rather than
hammer a free service. Caveats: our HU blueprint is small (only 6.3 M infosets, `local` menus, 700 M
iterations), and 6-max-tuned B itself was never trained heads-up.

## 5. Reproduce

```
scripts/dev.sh run 'cargo build --release -p bench -p pluribus'          # DEV_TARGET=extbots for a private target dir
DEV_TARGET=extbots scripts/dev.sh run 'cargo test --release -p bench --lib -- chart:: ext:: slumbot::'
# RLCard (dev box): ~/extbots/rlcard-env/venv (rlcard 1.2.0, torch cpu), then
#   ~/extbots/rlcard-env/venv/bin/python -I scripts/extbots/rlcard_train.py ~/extbots/rlcard-models/dqn6 90 4
scripts/extbots/run_rules.sh        # B vs chart6* / built-ins, mixed rule tables
scripts/extbots/run_rlcard.sh       # B / chart6 / built-ins vs the RLCard DQN (DEALS=100000 for the final rows)
bench table --hero compact:B.plcp --sizing local --bucketer tables:data/buckets-200 --field chart6,chart6-loose,chart6-tight,rlcard:MODEL,tag
bench slumbot --policy compact:data/hu200/compact.plcp --sizing local --bucketer tables:data/buckets-200 --hands 2400 --pace-ms 400 --log data/extbots-results/slumbot-hu200.jsonl
```

## 6. Limits, and what would make the assurance stronger

* "Strongest bots" turned out to be mostly unobtainable: no public strong 6-max agent exists. Our external
  field is (1) a rule bot we wrote to the shape of published charts (not solver output; it is competent but
  exploitable and tuned by feel, with three style settings), (2) a weak off-the-shelf learned agent, (3) the
  built-in archetypes. Beating these says B has no gross leaks against sane human-style and bot-style
  play; it does not certify equilibrium quality.
* Stronger checks that need access we do not have: the GTO Wizard Benchmark key (HU, would give an
  AIVAT-corrected bb/100 against a bot 19 bb/100 better than Slumbot), and long Open Poker sessions
  (live 6-max bots of unknown strength).
* Worth doing next with the bridge already in place: wrap a locally trained slumbot2019/DecisionHoldem
  blueprint (HU) as an `ext:` bot, and replace the DQN by a longer or architecture-improved RLCard run (DMC)
  with betting history in the observation.
