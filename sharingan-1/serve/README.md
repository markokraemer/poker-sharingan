# sharingan-serve: the SHARINGAN-1 inference API

Ask the bot for a decision over HTTP. **Stateless**: every request carries the full public hand
history; the server replays it through `engine/nlhe`, validates every action, asks SHARINGAN-1
(compact blueprint + turn/river real-time search, the "final config") and returns a **legal** action.

State and action JSON follow [`docs/TABLE_API.md`](../../docs/TABLE_API.md): table seats, 2-char cards
(`"Ah"`), integer chips, history entries `{seat, type, to}`, and **every amount is a "to"** (the
player's total contribution on the current street after the action), never a "by".

| endpoint | auth | |
|---|---|---|
| `POST /v1/act` | Bearer | one decision |
| `POST /v1/act/batch` | Bearer | up to 64 decisions, per-item results |
| `GET /v1/info` | Bearer | model id, blueprint, sizing, search config, live stats |
| `GET /v1/health` | none | liveness |

**Public endpoint:** `https://arena.sharingan.poker/v1/act` (and `/v1/act/batch`, `/v1/info`). The bearer is any arena
player token (`sk_...`, from `POST /v1/players` or the first `sharingan submit`), 10 decisions/s per player. A self-hosted
server accepts such tokens with `--arena-auth-url http://ARENA` next to its own `--api-keys`.

## Run

```bash
# build (note: panic=unwind so a panicking search falls back to the blueprint instead of aborting the process)
cargo build --release -p sharingan-serve --config 'profile.release.panic="unwind"'

SHARINGAN_BLUEPRINT=/data/sharingan-1.plcp \      # compact PLCP1 file, mmapped once at startup
SHARINGAN_BUCKETS=data/buckets-200 \              # card abstraction tables (264 MB)
SHARINGAN_SIZING=medium \                         # must match how the blueprint was trained
SHARINGAN_API_KEYS=alice=sk-alice,bob=sk-bob \
./target/release/sharingan-serve                  # listens on 0.0.0.0:8080
```

Defaults mirror the production live bot (`arena/openpoker`): search only on the **turn and river**
(`SHARINGAN_SET=flop=0`), 5 s hard budget, 16 search threads, `--max-stack-bb 150`.
Every setting is an env var or a `--flag` (flags win):

| env | flag | default | |
|---|---|---|---|
| `SHARINGAN_BLUEPRINT` | `--blueprint` | required | compact PLCP1 policy |
| `SHARINGAN_BUCKETS` | `--buckets` | `data/buckets-200` | `coarse` for toy blueprints |
| `SHARINGAN_SIZING` | `--sizing` | `medium` | `tiny`, `local`, `medium`, `pluribus` |
| `SHARINGAN_SET` | `--set` | `flop=0` | `k=v,...` overrides of the search (same keys as `openpoker --set`: `iters`, `flop`, `turn`, `river`, `pre`, `ms`, ...) |
| `SHARINGAN_NO_SEARCH` | `--no-search` | off | blueprint only |
| `SHARINGAN_BUDGET_MS` | `--budget-ms` | 5000 | hard per-request search budget |
| `SHARINGAN_SEARCH_THREADS` | `--search-threads` | 16 | one shared rayon pool |
| `SHARINGAN_SEARCH_CONCURRENCY` | `--search-concurrency` | 2 | searches running at once |
| `SHARINGAN_QUEUE_MS` | `--queue-ms` | 2000 | max wait for a search slot, then blueprint |
| `SHARINGAN_MAX_STACK_BB` | `--max-stack-bb` | 150 | never commit more than this many bb in one hand (0 = off) |
| `SHARINGAN_BIND` | `--bind` | `0.0.0.0:8080` | |
| `SHARINGAN_API_KEYS` / `_FILE` | `--api-keys` / `--api-keys-file` | required | `name=key,...`; or `--no-auth` for an open server |
| `SHARINGAN_RATE_RPS` / `_BURST` | `--rate-rps` / `--rate-burst` | 200 / 400 | token bucket per key (default for every key) |
| `SHARINGAN_RATE_LIMITS` | `--rate-limits` | none | per-key overrides by key name: `alice=1000:2000,bob=50` (`rps[:burst]`, burst defaults to 2 x rps) |
| `SHARINGAN_ARENA_AUTH_URL` | `--arena-auth-url` | off | also accept arena player tokens: checked once via `GET {url}/v1/me`, cached 5 min (rejections 30 s) |
| `SHARINGAN_PUBLIC_RPS` / `SHARINGAN_PUBLIC_BURST` | `--public-rps` / `--public-burst` | 10 / 20 | rate limit per arena player |
| `SHARINGAN_MAX_BATCH` | `--max-batch` | 64 | |
| `SHARINGAN_MAX_INFLIGHT` | `--max-inflight` | 256 | concurrent decisions; more -> 503 |
| `SHARINGAN_MODEL_ID` | `--model-id` | `sharingan-1` | returned in every response |
| `SHARINGAN_WARM` | `--warm` | off | read the whole blueprint ahead (only if it fits in RAM) |
| `SHARINGAN_LOG` | | on | `off` silences the JSON logs |

Logs are JSON lines on stderr (`startup`, `act`, `batch`, `auth`, ...) with request id, key name,
status, latency, source, street, action and fallback reason. Hole cards are never logged.

## Request

```bash
curl -s -X POST http://localhost:8080/v1/act \
  -H "Authorization: Bearer sk-alice" -H "content-type: application/json" -d '{
  "hand_id": "h-77",
  "small_blind": 50, "big_blind": 100, "button_seat": 3, "hero_seat": 0,
  "hole_cards": ["Ah", "Kd"],
  "board": ["7c", "8d", "2s", "Jh"],
  "players": [
    {"seat": 0, "stack_start": 10000}, {"seat": 1, "stack_start": 10000}, {"seat": 2, "stack_start": 10000},
    {"seat": 3, "stack_start": 10000}, {"seat": 4, "stack_start": 10000}, {"seat": 5, "stack_start": 10000}
  ],
  "history": [
    {"seat": 4, "type": "post_sb", "to": 50}, {"seat": 5, "type": "post_bb", "to": 100},
    {"seat": 0, "type": "raise", "to": 250},
    {"seat": 1, "type": "fold"}, {"seat": 2, "type": "fold"}, {"seat": 3, "type": "fold"}, {"seat": 4, "type": "fold"},
    {"seat": 5, "type": "call"},
    {"seat": 5, "type": "check"}, {"seat": 0, "type": "check"},
    {"seat": 5, "type": "bet", "to": 200}
  ],
  "options": {"seed": 1}
}'
```

* `stack_start` = chips behind **before** the blinds. `in_hand: false` marks a player sitting out.
  2 to 6 players are dealt in (3-5 are embedded in the 6-max tree, heads-up is played natively with
  the button as the small blind, as in TABLE_API).
* Seat order: the small blind is the first dealt-in seat after `button_seat` (the button seat may be empty).
* `history`: every action so far, in order. Leading `post_sb` / `post_bb` entries are optional (validated if present).
  Types: `fold`, `check`, `call`, `bet`, `raise` (needs `to`), `all_in` (to optional). `street` is optional and checked.
  A fold when checking is free is accepted as a check (TABLE_API 6.6) and reported in `notes`.
* `board` must have exactly the cards of the street the history ends on (0, 3, 4, 5).
* The hero must be the player on the clock. Antes and straddles are not supported.
* `options` (all optional): `include_probs` (default true), `time_budget_ms` (capped at the server budget),
  `seed` (reproducible sampling), `sample` (true = sample the mixed strategy, false = most likely action),
  `search` (false = blueprint only for this request).

## Response

```json
{
  "request_id": "r-1a180b05dbf-1",
  "hand_id": "h-77",
  "model_id": "sharingan-1",
  "street": "turn",
  "action": {"type": "fold", "added": 0, "all_in": false},
  "source": "blueprint",
  "fallback_reason": "off_policy_history",
  "probs": [
    {"type": "fold", "p": 0.909804}, {"type": "raise", "to": 1150, "p": 0.066667},
    {"type": "call", "to": 200, "p": 0.019608}, {"type": "raise", "to": 9750, "p": 0.003922}
  ],
  "legal": {"fold": true, "check": false, "call": {"to": 200, "added": 200, "all_in": false}, "raise": {"min_to": 400, "max_to": 9750}},
  "timing": {"total_ms": 11.4, "search_ms": null, "budget_ms": 5000},
  "notes": []
}
```

(Real output of the request above on the tiny test blueprint. The hero's preflop raise with AKo has ~zero likelihood
under that blueprint, so the turn search is skipped and the answer is the blueprint's, with its quantised
probabilities (`0.909804`, `0.066667`, ...). An answer that really comes from the search has `"source": "search"`,
`"fallback_reason": null`, a `search_ms`, and non-quantised probabilities.)

* `action`: `fold` | `check` | `call` (`to`, `added`) | `raise` (`to`, `added`; also used for a bet). Always legal in the caller's chips.
* `source`: `search` only when the answer comes from the turn/river search average; `blueprint` (no search needed:
  preflop/flop, or search disabled, or the search was skipped/empty, see below), `fallback` (search timed out / slot
  busy / panicked, or the hand could not be mapped; `fallback_reason` says why).
  A fallback still returns the blueprint action, or a passive legal action if the blueprint cannot be used.
* `source: "blueprint"` with a `fallback_reason` on a turn/river spot:
  * `off_policy_history`: the hero's earlier actions have ~zero likelihood under the hero's own strategy (typically
    a replayed foreign hand), so the subgame has no mass for the hero's hand. The search is **skipped** (instant answer).
  * `no_search_mass`: a search ran, but its average strategy has nothing for the hero's hand; the answer is the blueprint's.
* `probs`: the bot's distribution over real legal actions (before the stack guard); `notes`: lenient history repairs and
  stack-guard rewrites.

### Batch

```bash
curl -s -X POST http://localhost:8080/v1/act/batch -H "Authorization: Bearer sk-alice" \
  -d '{"requests": [ {...act request...}, {...act request...} ]}'
# -> {"request_id","count":2,"errors":1,"results":[{"ok":true,"response":{...}},{"ok":false,"error":{"code":"bad_amount",...}}]}
```

HTTP 200 whenever the batch itself is well-formed; each item succeeds or fails alone, in order. A batch
costs `n` rate-limit tokens (capped at the burst). Items run concurrently up to the search concurrency.

### Info / health

```bash
curl -s localhost:8080/v1/health
curl -s -H "Authorization: Bearer sk-alice" localhost:8080/v1/info
```

`/v1/info` reports the model id, blueprint (path, infosets, size), sizing, the search config (iterations, which
streets, budgets, threads, concurrency), the stack guard, limits, the panic strategy and live counters
(requests, searches, timeouts, busy, fallbacks, search latency).

## Errors

`{ "error": { "code", "message", "details" } }` (same envelope as TABLE_API 3). 4xx for the caller, 5xx only for us.

| status | code | meaning |
|---|---|---|
| 400 | `bad_request` | not valid JSON / wrong field types (chips must be integers) |
| 401 | `unauthorized` | missing / unknown Bearer key |
| 429 | `rate_limited` | per-key bucket empty (`Retry-After` header) |
| 413 / 400 | `bad_batch` | batch empty or larger than `max_batch` |
| 422 | `illegal_action` | e.g. check facing a bet, raise when no raise is possible, `bet` over an open bet |
| 422 | `bad_amount` | `to` outside `[min_to, max_to]` (details: `index`, `min_to`, `max_to`) |
| 422 | `missing_amount`, `unknown_action_type`, `bad_post` | malformed history entry (`details.index`) |
| 422 | `out_of_turn`, `unknown_seat`, `street_mismatch`, `action_after_hand_end` | inconsistent history |
| 422 | `not_hero_turn` | someone else is to act (`details.to_act_seat`) |
| 422 | `board_mismatch`, `invalid_card`, `duplicate_card` | cards do not fit the street / are malformed / repeat |
| 422 | `bad_blinds`, `bad_seat`, `duplicate_seat`, `bad_stack`, `bad_player_count`, `hero_not_in_hand`, `bad_budget` | bad table setup |
| 409 | `hand_over` | the history already ends the hand |
| 503 | `overloaded` | more than `max_inflight` decisions running |
| 500 | `internal` | bug (a panic is caught and reported, never crashes the worker) |

## How it works

* **Stateless replay.** `replay.rs` rebuilds two engines in lockstep. The *real* engine (your chips, your blinds) is the
  authority for legality: it validates every history action and every answer is clamped to its legal set. The *scaled*
  engine is what the bot sees: chips x `100/big_blind` (the blueprint was trained at 50/100, 100 bb) and, for 3-5 players,
  `6-n` phantom folders (same trick as `arena/openpoker/src/layout.rs`). The bot's `SearchPlayer` is fed the same
  `observe` / `observe_board` calls as in a live session (so beliefs and the off-tree shadow history are exact).
* **Blueprint** is a PLCP1 file, mmapped once, lock-free reads shared by all requests.
* **Search** (turn/river, `flop=0`) runs on a worker thread against a per-request clone of the player under two layers of
  time protection, as in `arena/openpoker/src/guard.rs`: the solver's cooperative cap (70% of the budget) and a hard
  `recv_timeout(budget)`. On timeout, panic or no free search slot the answer is the blueprint action. A timed-out search
  keeps its slot until it really ends, so abandoned work cannot pile up. Worst-case latency is
  `queue_ms + budget_ms` (blueprint-only paths answer in about a millisecond).
* **Stack guard** (`--max-stack-bb 150`): a raise/all-in that would commit more than 150 bb in one hand is rewritten to a
  raise to exactly the cap, else a call, else a check (same rule as `mapping::cap_action`).
* **Auth / limits**: `Authorization: Bearer <key>`, per-key token bucket, 8 MB body limit, bounded in-flight decisions.

## Tests

Tests run against a **tiny blueprint** (6-max, `--sizing tiny`, 2M iterations, ~0.26 GB compact):

```bash
cargo build --release -p pluribus
./target/release/pluribus train --out tinybp --iters 2000000 --sizing tiny --players 6 --mem-gb 8   # ~30 s
./target/release/pluribus compact --ckpt tinybp/blueprint.bin --sums tinybp/snapshots.sum --out tinybp/tiny.plcp --threads 8

SHARINGAN_TEST_BLUEPRINT=$PWD/tinybp/tiny.plcp cargo test -p sharingan-serve   # default: ~/dev-serve/tinybp/tiny.plcp
```

(needs `data/buckets-200`; without the blueprint file the integration tests print `SKIPPED`.)

* `tests/api.rs`: 4000 random hands (2-6 players, any button, short/deep stacks, bb in {1,2,10,20,25,30,50,100,200,1000})
  checked against an independent nlhe oracle for legality of the action, the `legal` block and every prob entry;
  HTTP path and seed determinism; turn/river search runs and flop does not; **time-budget fallback** (a search forced to
  outlast a 300 ms budget answers from the blueprint in time, the next search sees `search_busy`, the slot is released
  afterwards); batch (mixed good/bad items, limits); auth and rate limit; deep-stack guard; TABLE_API semantics
  (min-raise chain, short all-in does not reopen).
* `tests/fuzz.rs`: targeted illegal histories with exact expected codes (bad amounts, check facing a bet, wrong seat,
  street labels, board/cards, bad players/blinds), malformed bodies (400), and 2500 randomly corrupted requests that must
  never 5xx and, when still accepted, must answer legally.
* unit tests in `src/` (config parsing, replay, seat order, scaling, guard).

## Serving the real (~140 GB) blueprint

* Point `SHARINGAN_BLUEPRINT` at the final compact PLCP1 file and set `SHARINGAN_SIZING=medium` (blueprint C) with
  `SHARINGAN_BUCKETS=data/buckets-200`. Nothing else changes; the file is mmapped, never loaded into the heap.
* RAM: the file should live in the page cache (140 GB on a 256+ GB box is fine). Cold, the first lookups page-fault from
  disk; pre-warm with `--warm` (madvise WILLNEED) or `cat file > /dev/null`. Several instances share the same page cache.
* Build with `panic="unwind"` (see above) in production; `/v1/info` shows `panic_strategy`.
* Terminate TLS in front (nginx/Caddy/ALB); the server speaks plain HTTP.

## Known gaps

* **Earlier-street search is not replayed.** The hero's range belief is Bayes-updated with the *blueprint* likelihood of the
  hero's own earlier actions; a live session would use the search average strategy of rounds it actually searched (only
  matters on the river after a searched turn). Re-solving earlier rounds per request would cost another search each.
* **Chip scaling is exact only when `big_blind` divides 100** (1, 2, 4, 5, 10, 20, 25, 50, 100). Other blinds are rounded;
  in rare near-all-in situations the scaled and real engines can disagree, and the request then gets a passive legal action
  with `source: "fallback"` and `fallback_reason: "desync..."` (1 in 4000 random hands in the tests).
* Heads-up is outside the 6-max training distribution; the blueprint has no matching infosets for a 2-player history and falls back to check/call there (untested for quality).
* No antes / straddles, no per-opponent modelling (the bot is the equilibrium strategy), no WebSocket / webhook mode
  (TABLE_API sections 4 and 11 are for the arena server, this is the stateless decision endpoint).
* Rate limiting and key state are in-memory per process.
