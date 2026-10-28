# openpoker: live 6-max evaluation on Open Poker

Plays our bench policies (the compact blueprint, or built-ins for smoke tests) on
[Open Poker](https://openpoker.ai), a free 6-max bot arena (WebSocket JSON, blinds 10/20, buy-in
1,000-5,000 chips, other developers' bots as opponents). Every decision is traced; a history
analyzer turns traces into bb/100 with confidence intervals and an all-in-luck adjustment.

**Everything here runs on the dev box, never on the Mac** (`scripts/dev.sh`, see below). Do not
run large live sessions without a human deciding to.

## Setup (once)

Get an API key from Open Poker and keep it out of the repo. Put it in an env file with mode 600
(`OPENPOKER_API_KEY=...`), never commit it and never print it:

```bash
scp -o IdentitiesOnly=yes my-openpoker.env ubuntu@$PLURIBUS_DEV_HOST:~/pluribus/.env.openpoker
ssh ubuntu@$PLURIBUS_DEV_HOST 'chmod 600 ~/pluribus/.env.openpoker'
```

`openpoker` reads `OPENPOKER_API_KEY` from the environment, else from `--env FILE`, else
`./.env.openpoker`, `~/pluribus/.env.openpoker`, `./.env`. `scripts/dev.sh` rsyncs the workspace
without `--delete`, so the file survives syncs.

## Build and test (dev box)

```bash
DEV_TARGET=openpoker scripts/dev.sh run 'cargo test --release -p openpoker'
DEV_TARGET=openpoker scripts/dev.sh run 'cargo build --release -p openpoker'
```

34 tests: protocol parsing, layout/seat rotation, action-mapping legality fuzz (200k random
offered-sets), replay of hand-crafted and **real recorded messages**
(`tests/fixtures/smoke_raw.jsonl`, a 10-hand live session replayed bit-exactly), session edge
cases (duplicate turns, rejects, resync, bust/rebuy, mid-hand joins), all-in equity, analyzer math.

## Run

```bash
B=target/openpoker/release/openpoker
# smoke: <= 10 hands with a built-in
$B live --policy builtin:tag --hands 10 --buy-in 2000 --timeout-sec 600 --trace logs/openpoker-smoke.jsonl
# the blueprint (compact file from the eval pipeline), 100 bb buy-in
nohup $B live --policy compact:data/eval/B/compact.plcp --sizing paper --buckets data/buckets-200 \
      --hands 10000 --buy-in 2000 --timeout-sec 86400 --trace logs/openpoker-B.jsonl > logs/openpoker-B.out 2>&1 &
# analysis of any trace (also works while a session is running)
$B analyze --trace logs/openpoker-B.jsonl [--json] [--mc 20000]
# server-side history (lossy, see caveats) and season snapshot
$B export --out logs/op-history.json --limit 500 && $B analyze --history logs/op-history.json
$B season
```

| flag | meaning |
|---|---|
| `--policy` | `builtin:NAME` (random/station/nit/tag/lag/maniac), `compact:PATH`, `bp:PATH` (checkpoint) |
| `--sizing`, `--buckets` | blueprint bet-size preset (`bench::bp::sizing_by_name`) and bucket-table dir (`tables:DIR`) or `coarse`; must match how the blueprint was trained |
| `--hands N` | stop after N completed hands (default 10) |
| `--buy-in` | chips on joining (1000-5000; 2000 = 100 bb, comparable to the paper's stacks) |
| `--timeout-sec` | wall-clock cap for the whole session (default 3600); leaves the table on expiry |
| `--trace FILE` | JSONL trace (default `logs/openpoker-<ms>.jsonl`); `--no-raw` drops the raw message log |
| `--hu fallback\|policy` | who plays heads-up hands (default `fallback`, `--fallback builtin` name, default `tag`) |
| `--no-auto-rebuy` | handle `busted` ourselves (rebuy then rejoin) instead of server auto-rebuy |
| `--max-stack-bb X` | default 150, 0 = off. Stacks accumulate on Open Poker but the blueprint is trained at 100 bb. (1) after a hand, if our stack > X bb, `leave_table` then `join_lobby` with `--buy-in` (excess stays in the season balance); (2) guard: never commit more than X bb in one hand (raise/all-in beyond it becomes a raise to the cap, else a call) |
| `--seed`, `--max-reconnects`, `--ws-url`, `--api-url` | determinism and plumbing |

## How the game is mapped

* **Chips.** Their 10/20 blinds become our 50/100: every amount is x5 going in, rounded and clamped
  to the advertised `[min, max]` raise-to going out. Per-seat stacks are exact (`GameConfig::with_stacks`).
* **Seats.** nlhe seat 0 = SB, 1 = BB, last = BTN. Open Poker's SB is the first dealt-in seat after
  `dealer_seat`. Heads-up (observed live) the **dealer is the BB**; the other player is the SB.
* **State.** The server never sends a full hand state, so we replay every `player_action` on an
  `nlhe::State`. Start-of-hand stacks come from the first action's `stack_before` plus the blind
  that seat posted (seats that have not acted: current stack plus blind). At each `your_turn`
  the engine's pot, stacks and legal actions are cross-checked against the server's; any
  disagreement lands in the decision's `issues`.
* **Who is dealt in** is not stated by the server (`players` has no status), so the replay tries
  the plausible sets (largest first, excluding mid-hand joiners) and keeps one whose replay is
  consistent with the event order and the server pot. If a later event contradicts it, the replay
  is rebuilt from the full event list.
* **Fewer than 6 players.** With a blueprint (`compact:`/`bp:`) 3-5 player hands are embedded in
  the 6-max game: the `6-n` earliest non-blind seats are phantoms that fold immediately, so the
  blueprint's history looks like "folded around" and real players keep their true order.
  Built-ins use a native n-player engine. **Heads-up cannot be embedded**, so by default HU hands
  are played by the fallback built-in (`--hu fallback`, flagged `fallback:tag` in the trace);
  `--hu policy` feeds the blueprint a native 2-player game, which is out of distribution for it.
* **Our action -> theirs.** `Fold/Check/Call` map directly; `RaiseTo(t)` becomes `raise` with
  `round(t/5)` clamped into the offered range (`all_in` if only a short all-in is possible);
  anything not offered degrades to check/fold/call. The trace records fidelity (`Exact`,
  `Adjusted(wanted,sent)`, `Degraded`).
* **Failures.** An unplaceable event, a policy error or a missing board marks the hand `desync`
  and answers with the safest legal action (check, else fold). A rejected action is retried once
  with that action (`invalid_action` only). Duplicate `turn_token`s and `table_seq` regressions
  are ignored.
* **Reconnect.** On socket loss: reconnect with backoff; `GET /api/me/active-game`; if playing,
  `resync_request` (the replayed events are fed through the normal handlers and a snapshot that
  carries our `turn_token` is turned back into a `your_turn`), else `join_lobby`. A resync does not
  extend the server's 45 s turn timer; our policies answer in microseconds. A resync into a hand
  we never saw start cannot be replayed (no events) and falls back to the safe action.
* **Sitting out.** There is no sit-out message; we never sit out. `busted` -> server auto-rebuy
  (1,500 chips; buy-in differs from `--buy-in` afterwards, bb/100 normalises this), `table_closed`
  / `season_ended` -> rejoin the lobby.

## Trace (JSONL, one flushed line per record)

`session_start`, `raw` (every message in/out, includes `turn_token` but never the API key),
`decision` (server legal set, engine/server `issues`, **full distribution**, sampled `u`, chosen
nlhe action, mapped Open Poker action + fidelity, fallback reason, latency), `ack`, `reject`,
`note`, `anomaly`, `hand` (the analyzer's input: players with start stacks, contributions,
shown cards, all actions, profit and its source, `lock_len`, counters), `session_end`.

## What the numbers mean

* **bb/100** = mean hand profit in big blinds x 100, from `final_stacks` (authoritative; the
  payouts exclude refunds of uncalled bets). 95% CI is Student-t on per-hand results; hands are
  treated as independent although they share tables and stacks.
* **all-in adj** replaces each hand that ended in an all-in run-out (board incomplete when betting
  stopped) and where every live player's cards are known (our own plus `shown_cards`) with its
  expectation (exact enumeration for flop/turn locks, 20k samples preflop; side pots handled).
  This removes **run-out luck only**. `luck removed` shows how much it moved the mean;
  `variance ratio` how much per-hand variance it removed (typically 1.3-2x).
* **Cannot be corrected:** luck in hands that are not all-in before the river (folded/mucked cards
  are never revealed), hole-card luck, and everything AIVAT would remove: opponent strategies are
  unknown, so there is no action-based correction. If a loser mucks after a run-out, the
  adjustment skips that hand, which can bias it slightly; the server normally shows all-in cards.
* **Sample size.** Typical 6-max per-hand sd is ~10 bb, so the 95% CI half-width is about
  +-62 bb/100 at 1k hands, +-20 at 10k, +-6 at 100k. Anything under ~10k hands is a health check
  (protocol, legality, gross leaks), not a strength measurement. The report prints "hands
  needed for +-5 bb/100" at the observed sd.
* **Breakdowns** by position, table size, policy used (so heads-up fallback hands do not pollute
  the blueprint number) and by opponent present (overlapping; most useful to spot one bot you
  lose to or a calling station you farm).

## Caveats

* **The opponent field is unknown and shifts every season** (other developers' bots, LLM bots,
  built-in templates such as Gambler/Calling Station). Winning here is exploitation of that
  field, not distance from equilibrium; losing to it is more informative than winning.
* Stacks are **not 100 bb**: opponents range from 15 bb to 2,700 bb (seat with 55k chips seen),
  tables are often 2-4 handed, and the hero's own stack drifts above/below the buy-in. The
  blueprint was trained at 100 bb 6-max, so deep/short effective stacks and short-handed tables
  are partly out of distribution. Read the by-table-size rows.
* Their minimum raise equals ours (x5) and bet sizes are translated with the policy's own
  pseudo-harmonic translation; the blueprint's sizing preset must match training.
* `export`/`--history` is **lossy**: call amounts are 0, street labels are off by one (they name
  the street *after* the closing action), `pot_total` disagrees with the actions, no opponent
  names. Only hands that replay cleanly are all-in-adjusted. Prefer our own trace.
* Live server quirks found while building this (all handled): `all_in` broadcasts carry the
  pre-action stack/pot; `fold` is listed even when checking is free; `winners[].amount` is net,
  `payouts[].amount` is gross; a player can join mid-hand (not dealt in); `players` in `your_turn`
  has no status; the dealer is the BB heads-up.
* Seasons reset on a calendar; compare runs within one season. Be considerate with volume
  (free tier, 20 msg/s limit; we send one message per decision).
