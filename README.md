<p align="center"><img src="assets/header.svg" width="100%" alt="POKER SHARINGAN"></p>

```text
11  AN AGENT-BUILT, FROM-SCRATCH REPRODUCTION OF PLURIBUS (BROWN & SANDHOLM, SCIENCE 2019),
12  THE FIRST AI TO BEAT PROFESSIONALS AT SIX-PLAYER NO-LIMIT HOLD'EM. ONE RUST WORKSPACE:
13  TRAINER, ABSTRACTION, REAL-TIME SEARCH, EVALUATION, LIVE CLIENT. CODE AND WEIGHTS OPEN.
```

### EXHIBIT A: RESULTS

```text
14  ALL NUMBERS: DUPLICATE DEALS + AIVAT, MBB/GAME (10 MBB/GAME = 1 BB/100), 95% CI.
15  BLUEPRINT@N = AFTER N MCCFR ITERATIONS. THREE BET-SIZE MENUS WERE TRAINED; C IS THE BEST.
16
17  MATCH                                          DEALS     RESULT
18  ---------------------------------------------  --------  -----------------
19  C@4.77e10 vs C@2e10       (3 + 3 seats)        4M        +3.8   +- 1.0
20  C@4.77e10 vs B@2e10       (3 + 3 seats)        4M        +3.4   +- 1.3
21  C@2e10    vs A@2e10       (3 + 3 seats)        4M        +4.0   +- 1.3
22  C@1.83e11 vs C@4.77e10    (3 + 3 seats)        4M        PENDING
23
24  SEARCH + C@2e10 vs 5 x C@2e10                  30k       +87    +- 27
25  C@4.77e10 vs 5 x SvanBot  (live budget)        20k       +56.7  +- 34.7
26  SvanBot   vs 5 x C@4.77e10 (live budget)       50k       -46.9  +- 21.2
27
28  EXPLOITER (TRAINED BEST RESPONSE, 12M ITERS)   DEALS     EXPLOITER WINS
29  ---------------------------------------------  --------  -----------------
30  vs C@4.77e10                                   20k       -290   +- 59
31  vs C@2e10                                      20k       -323   +- 62
32
33  LIVE, OPEN POKER ARENA (SEARCH + C@2e10)       HANDS     BB/100, LUCK-ADJ.
34  ---------------------------------------------  --------  -----------------
35  final config                                   2,329     +77.6  [26, 129]
```

Negative exploiter numbers mean the exploiter loses: it found no hole worth money. SvanBot ran with a fresh install's
defaults and no learned opponent model. It still beats weak rule bots harder than we do, which is a different game.
Every caveat is in [docs/RESULTS.md](docs/RESULTS.md).

### EXHIBIT B: HOW IT WORKS

```text
36  1  PLAY BILLIONS OF HANDS AGAINST ITSELF. AFTER EACH DECISION, RECORD HOW MUCH BETTER
37     EVERY OTHER ACTION WOULD HAVE DONE (REGRET). PLAY ACTIONS IN PROPORTION TO REGRET.
38  2  SAMPLE: THE LEARNER TRIES EVERY ACTION, EVERYONE ELSE AND THE DECK PLAY ONE LINE.
39  3  FORGET EARLY GARBAGE (LINEAR DISCOUNTING). STOP EXPLORING HOPELESS ACTIONS (PRUNING).
40  4  SHRINK THE GAME: 200 BUCKETS OF SIMILAR HANDS PER STREET, A FIXED MENU OF BET SIZES.
41  5  THE AVERAGE OF ALL THAT PLAY IS THE BLUEPRINT.
42  6  AT THE TABLE, ON TURN AND RIVER: GUESS EACH RANGE FROM THE BETTING, RE-SOLVE THE SPOT
43     A FEW ACTIONS DEEP, LET OPPONENTS SWITCH BETWEEN 4 STYLES AT THE HORIZON, PLAY IT.
```

The maths, in full: [docs/METHOD.md](docs/METHOD.md).

### EXHIBIT C: RUN IT

```bash
git clone https://github.com/markokraemer/poker-sharingan && cd poker-sharingan
RUSTFLAGS="-C target-cpu=native" cargo build --release

# weights + card abstraction (not in git)
huggingface-cli download TODO/sharingan --local-dir weights

# head-to-head, 6-max, duplicate + AIVAT
./target/release/pluribus h2h --a weights/C-4.77e10.plcp --sizing-a medium --b weights/C-2e10.plcp --sizing-b medium \
  --buckets data/buckets-200 --players 6 --deals 1000000 --threads 32 --warm --out h2h.json
```

The CLI is named `pluribus`, after the paper. Training, evaluation, search and the live client:
[docs/OVERVIEW.md](docs/OVERVIEW.md) · [docs/TRAINING.md](docs/TRAINING.md) · [docs/EVALUATION.md](docs/EVALUATION.md).

### EXHIBIT D: NEXT

```text
44  PLAY AGAINST IT     A PUBLIC 6-MAX TABLE API. BRING AN AGENT, TAKE A SEAT.      SOON
45  OPEN BENCHMARK      DUPLICATE + AIVAT LEADERBOARD. ANYONE CAN SUBMIT A BOT.      SOON
46  CONTRACT            docs/TABLE_API.md                                          DRAFT
```

### EXHIBIT E: THE FINE PRINT

```text
47  COMPUTE      ~EUR 480 SO FAR ON RENTED 64-CORE BOXES. FINAL RUN IN PROGRESS.
48  NOT PROVEN   NO MATCH AGAINST HUMAN PROFESSIONALS. NO MATCH AGAINST PLURIBUS ITSELF.
49  USE          RESEARCH AND BOT ARENAS. REAL-MONEY SITES BAN BOTS. DON'T.
50  LICENSE      APACHE-2.0. THIRD-PARTY NOTICES IN NOTICE.
```

<sub>Brown, N. & Sandholm, T. Superhuman AI for multiplayer poker. <i>Science</i> 365, 885–890 (2019). doi:10.1126/science.aay2400</sub>
