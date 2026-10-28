# Table API — contract for running the Pluribus bot (and any other bot or human) in 6-max NLHE

This is the contract the bot needs to play **flawlessly** and to be **measured rigorously**: AIVAT and duplicate
dealing, as in the paper. The server is the single source of truth. Clients (bots, human UIs) only send actions.

| Surface | Purpose |
|---|---|
| **REST** `/v1/...` | Players, tables, matches, standings, hand histories (create / read / export) |
| **WebSocket** `/v1/play` | The live game. The server pushes every event; the client answers `your_turn` with one action |
| **Webhook (optional)** | `POST` to a bot URL instead of the WebSocket (same payloads), for stateless bots |

All JSON is UTF-8, `snake_case`, with integer chips (no floats) and ISO-8601 UTC timestamps plus `t_ms` epoch millis.

---

## 1. Conventions

* **Cards**: 2 chars, rank `23456789TJQKA` and suit `cdhs`, e.g. `"Ah"`, `"Td"`, `"2c"`. Board is an array in deal order.
* **Seats**: integers `0..5`, fixed for the whole hand. Clockwise = increasing seat index (mod 6).
* **Button** moves one occupied seat clockwise every hand. **SB** = first occupied seat after the button, **BB** = next.
  Heads-up exception: the button posts the SB and acts first preflop.
* **Chips**: integers. Blinds are e.g. `sb=50, bb=100`. Every stack in a hand is reported in chips.
* **Amounts are always "to", never "by"**: `to` = the player's **total contribution on the current street** after the
  action. A preflop raise to 3bb is `to=300` whether the player posted a blind or not. The server also sends
  `added` (chips moved this action) for convenience. If both are sent, `to` wins.
* **Streets**: `preflop`, `flop`, `turn`, `river`, `showdown`.
* **Action types** (the full closed set):

| type | sent by | meaning |
|---|---|---|
| `post_sb`, `post_bb`, `post_ante`, `post_straddle` | server | forced bets (`to` = amount posted) |
| `fold` | client/server | |
| `check` | client/server | only when `to_call == 0` |
| `call` | client/server | match the current bet (or all-in for less). `to` = resulting street contribution |
| `bet` | client | first voluntary chips on a postflop street (alias of `raise` when `current_bet == 0`) |
| `raise` | client | raise to `to` (≥ `min_raise_to`, ≤ `max_raise_to`) |
| `all_in` | server (echo) | the server **reports** any action that puts the player all-in with `all_in: true` on the event. Clients just send `raise`/`call` with the right `to` |

* **IDs**: opaque strings: `player_id`, `table_id`, `match_id`, `hand_id`, `action_id`.
* **Sequence numbers**: every event in a hand carries `seq` (1, 2, 3, … gap-free per hand) and every table event carries
  `table_seq` (monotonic per table). They are used for ordering, deduplication and resync.

---

## 2. Entities (server-side data model, all readable over REST)

```ts
type Player = {
  player_id: string; name: string;
  kind: "bot" | "human";
  owner?: string;                 // who runs it
  created_at: string;
};

type TableConfig = {
  variant: "nlhe";
  max_seats: 6;                   // we need 6; 2–9 allowed
  small_blind: number; big_blind: number;
  ante: number;                   // 0 for the Pluribus format
  straddle: false;
  starting_stack: number;         // 10000 = 100 bb
  stack_mode: "reset" | "carry";  // "reset" = every hand starts every player at starting_stack (REQUIRED for eval)
  decision_time_ms: number;       // per decision, e.g. 5000 (we need >= 3000)
  time_bank_ms: number;           // extra per-player bank, e.g. 30000 (0 = none)
  timeout_action: "check_fold";   // check if free, else fold
  rake: { percent: 0, cap: 0 };   // 0 for eval
  min_players_to_deal: 6;         // eval: only deal full tables
  odd_chip_rule: "first_left_of_button";
  show_mode: "standard";          // last aggressor shows first; others may muck (server still logs all cards)
};

type Table = {
  table_id: string; config: TableConfig;
  status: "waiting" | "running" | "paused" | "closed";
  seats: Seat[];                  // length max_seats
  button_seat: number | null;
  hand_id: string | null;         // current hand
  table_seq: number;
};

type Seat = {
  seat: number;
  player_id: string | null; name: string | null;
  stack: number;                  // between hands; during a hand see HandState
  status: "empty" | "seated" | "sitting_out" | "disconnected";
};

type Match = {                    // a planned series of hands = one experiment
  match_id: string; table_id: string;
  participants: string[];         // player_ids, length 6
  hands_planned: number;          // e.g. 10000
  duplicate: DuplicateConfig | null;
  seed: string;                   // master seed (hex); the whole match is reproducible from it
  status: "scheduled" | "running" | "finished" | "aborted";
  hands_played: number;
  created_at: string; started_at?: string; finished_at?: string;
};

type DuplicateConfig = {
  enabled: true;
  rotations: 6;                   // each deal replayed 6× with seat assignment rotated so every participant
                                  // sits in every seat for every deal (1 deal = 6 hands)
  deck_from: "seed";              // deck_i = shuffle(HMAC_SHA256(seed, "deal:" + i)), Fisher-Yates (spec §7)
};
```

---

## 3. REST endpoints

Auth: `Authorization: Bearer <token>`. Tokens are per player (bot) or per admin.

| method | path | body / query | returns |
|---|---|---|---|
| `POST` | `/v1/players` | `{name, kind}` | `Player` + `token` (shown once) |
| `GET` | `/v1/players/{player_id}` | | `Player` + lifetime stats |
| `POST` | `/v1/tables` | `TableConfig` | `Table` |
| `GET` | `/v1/tables/{table_id}` | | `Table` + the current public `HandState` |
| `POST` | `/v1/tables/{table_id}/seats/{seat}` | `{player_id}` | `Seat` (sit down; admin or the player) |
| `DELETE` | `/v1/tables/{table_id}/seats/{seat}` | | stand up (takes effect after the current hand) |
| `POST` | `/v1/matches` | `{table_config, participants[6], hands_planned, duplicate?, seed?}` | `Match` (server creates and seats the table) |
| `POST` | `/v1/matches/{match_id}/start` / `pause` / `abort` | | `Match` |
| `GET` | `/v1/matches/{match_id}` | | `Match` + `standings` (§9) |
| `GET` | `/v1/matches/{match_id}/hands` | `?after_hand_no=&limit=500` | `HandHistory[]` (§8), paginated, **all hole cards** |
| `GET` | `/v1/matches/{match_id}/hands.jsonl` | | full export, one `HandHistory` per line (streamed) |
| `GET` | `/v1/hands/{hand_id}` | | `HandHistory` |
| `GET` | `/v1/hands/{hand_id}/events` | | the exact WebSocket event list of that hand (for replay or debug) |

Errors are `{ "error": { "code": "...", "message": "...", "details": {...} } }` with an HTTP status (§10).

---

## 4. WebSocket `/v1/play`: the live protocol

Connect with `wss://host/v1/play?token=...`. One connection = one player. The player may sit at several tables;
every message carries `table_id`.

### 4.1 Envelope (every message, both directions)

```ts
type Envelope = {
  v: 1;                           // protocol version
  type: string;                   // message type below
  table_id?: string; hand_id?: string;
  seq?: number;                   // per-hand event seq (server -> client)
  table_seq?: number;             // per-table seq (server -> client)
  t_ms: number;                   // server time (or client send time)
  data: object;
};
```

### 4.2 Client → server

| type | data | when |
|---|---|---|
| `hello` | `{client: "pluribus/0.1", resume?: [{table_id, last_table_seq}]}` | first message; with `resume` after a reconnect |
| `act` | `{hand_id, decision_id, type, to?}` | answer to `your_turn` (exactly one per `decision_id`) |
| `ping` | `{}` | keep-alive every 15 s (the server answers `pong`) |
| `leave` | `{table_id}` | stand up after the current hand |

`act` rules: `decision_id` must echo `your_turn.decision_id`. A duplicate `act` with the same `decision_id` is
ignored (idempotent). `to` is required for `raise`/`bet`. For `call` it is optional; the server computes it.

### 4.3 Server → client

| type | data | notes |
|---|---|---|
| `welcome` | `{player_id, name, server_time, tables: [table_id], protocol: 1}` | after `hello` |
| `table_snapshot` | `Table` + `hand: HandState \| null` | after `welcome`, after every reconnect, and on request; **full state, client must replace its model** |
| `hand_start` | `HandStart` (§5.1) | deal begins |
| `hole_cards` | `{hand_id, seat, cards: ["Ah","Kd"]}` | only to that player (and to admin/observer streams) |
| `action` | `ActionEvent` (§5.2) | every action incl. blinds, timeouts and auto-folds |
| `street` | `{hand_id, street, board: [...all board cards so far], new_cards: [...], pot, pots}` | flop/turn/river dealt |
| `your_turn` | `YourTurn` (§5.3) | it is this player's decision; deadline included |
| `hand_end` | `HandEnd` (§5.4) | showdown + results; **includes every player's hole cards** (§5.4 note) |
| `seat_update` | `Seat` | join/leave/sit-out between hands |
| `match_update` | `{match_id, hands_played, standings}` | every N hands |
| `error` | `{code, message, decision_id?, details?}` | e.g. invalid action (the player gets a fresh `your_turn` if time remains) |
| `pong` | `{}` | |

Ordering guarantee: per table, messages arrive in `table_seq` order. Per hand, `seq` is gap-free. A client that sees
a gap must request a resync with `hello{resume}`, or `GET /v1/tables/{id}` for the snapshot.

---

## 5. Hand payloads

### 5.1 `hand_start`

```ts
type HandStart = {
  hand_id: string; hand_no: number;            // hand_no: 1.. within the match
  match_id: string | null;
  deal_no: number | null; rotation: number | null;   // duplicate: deal index and rotation 0..5
  button_seat: number; sb_seat: number; bb_seat: number;
  small_blind: number; big_blind: number; ante: number;
  players: Array<{
    seat: number; player_id: string; name: string;
    stack_start: number;                       // chips BEFORE blinds/antes
    in_hand: boolean;                          // false = sitting out this hand
  }>;
  deck_commitment: string;                     // sha256(deck order + salt), revealed at hand_end (§7)
};
```

### 5.2 `action` (an event, broadcast to everyone at the table)

```ts
type ActionEvent = {
  hand_id: string; seq: number; action_id: string;
  street: "preflop" | "flop" | "turn" | "river";
  seat: number;
  type: "post_sb" | "post_bb" | "post_ante" | "fold" | "check" | "call" | "bet" | "raise";
  to: number;                 // player's total street contribution after the action (0 for fold/check)
  added: number;              // chips moved by this action
  all_in: boolean;
  forced: boolean;            // blinds/antes
  timeout: boolean;           // server acted on the player's behalf
  // state AFTER the action (lets clients verify their own engine every step):
  stack: number;              // this player's remaining stack
  pot: number;                // total chips in the middle incl. current street bets
  current_bet: number;        // highest street contribution
  min_raise_to: number;       // for the next raiser
  next_to_act: number | null;
};
```

### 5.3 `your_turn` (the only message a bot must answer)

```ts
type YourTurn = {
  hand_id: string; seq: number;
  decision_id: string;              // echo in `act`
  seat: number;
  street: "preflop" | "flop" | "turn" | "river";
  hole_cards: [string, string];     // repeated for stateless bots
  board: string[];
  // Full public history of THIS hand, so a stateless bot needs nothing else:
  history: ActionEvent[];           // every action so far incl. blinds, in seq order
  players: Array<{ seat: number; name: string; stack_start: number; stack: number;
                   street_contrib: number; hand_contrib: number;
                   status: "active" | "folded" | "all_in" | "out" }>;
  button_seat: number; small_blind: number; big_blind: number;
  pot: number;                      // total incl. current street bets
  pots: Array<{ amount: number; eligible_seats: number[] }>;  // main + side pots so far
  legal: {
    fold: boolean;                  // false only when checking is free and fold is disallowed (see rules)
    check: boolean;
    call: { to: number; added: number; all_in: boolean } | null;
    raise: { min_to: number; max_to: number } | null;   // max_to = all-in; null if raising is impossible
  };
  deadline_ms: number;              // absolute epoch ms; act before this
  time_bank_ms: number;             // remaining personal bank
};
```

Example (seat 0 on the flop, facing a check):

```json
{"v":1,"type":"your_turn","table_id":"t1","hand_id":"h-77","seq":9,"t_ms":1792000000000,
 "data":{"decision_id":"d-77-9","seat":0,"street":"flop","hole_cards":["Ah","Kd"],
  "board":["7c","8d","2s"],
  "history":[
   {"seq":1,"street":"preflop","seat":4,"type":"post_sb","to":50,"added":50,"all_in":false,"forced":true,"timeout":false,"stack":9950,"pot":50,"current_bet":50,"min_raise_to":100,"next_to_act":5},
   {"seq":2,"street":"preflop","seat":5,"type":"post_bb","to":100,"added":100,"all_in":false,"forced":true,"timeout":false,"stack":9900,"pot":150,"current_bet":100,"min_raise_to":200,"next_to_act":0},
   {"seq":3,"street":"preflop","seat":0,"type":"raise","to":250,"added":250,"all_in":false,"forced":false,"timeout":false,"stack":9750,"pot":400,"current_bet":250,"min_raise_to":400,"next_to_act":1},
   "... folds ...",
   {"seq":8,"street":"flop","seat":5,"type":"check","to":0,"added":0,"all_in":false,"forced":false,"timeout":false,"stack":9750,"pot":550,"current_bet":0,"min_raise_to":100,"next_to_act":0}],
  "players":[{"seat":0,"name":"pluribus","stack_start":10000,"stack":9750,"street_contrib":0,"hand_contrib":250,"status":"active"},
             {"seat":5,"name":"erin","stack_start":10000,"stack":9750,"street_contrib":0,"hand_contrib":250,"status":"active"},
             "..."],
  "button_seat":3,"small_blind":50,"big_blind":100,"pot":550,
  "pots":[{"amount":550,"eligible_seats":[0,5]}],
  "legal":{"fold":true,"check":true,"call":null,"raise":{"min_to":100,"max_to":9750}},
  "deadline_ms":1792000005000,"time_bank_ms":30000}}
```

Answer:

```json
{"v":1,"type":"act","table_id":"t1","t_ms":1792000000412,
 "data":{"hand_id":"h-77","decision_id":"d-77-9","type":"raise","to":400}}
```

### 5.4 `hand_end`

```ts
type HandEnd = {
  hand_id: string; seq: number;
  board: string[];                              // all 5 if dealt (also the cards that WOULD have come, see note)
  showdown: Array<{ seat: number; cards: [string,string]; shown: boolean; hand_rank: string; best5: string[] }>;
  hole_cards: Record<number, [string, string]>; // EVERY dealt player's cards, folded or not (see note)
  pots: Array<{ amount: number; eligible_seats: number[]; winners: number[]; share: number[] }>;
  results: Array<{ seat: number; player_id: string; net: number; stack_end: number }>;  // sum(net) == 0 (no rake)
  deck_reveal: { deck: string[]; salt: string };  // proves deck_commitment (§7)
  ended_by: "showdown" | "fold";
};
```

**Note: full information after the hand.** AIVAT, the variance reduction that makes ~10k hands meaningful instead of
~1M, needs all hole cards and the undealt runout once the hand is over. Send them in `hand_end` to every player.
A human-facing product can instead put them only in the REST hand history, as long as bots can read that within
a few seconds of the hand ending.

---

## 6. Game rules the server must implement (and the bot assumes)

1. Standard NLHE betting. A **raise must increase the bet by at least the last full bet or raise** on that street
   (preflop the first raise is at least 1 bb on top). `min_raise_to = current_bet + max(last_full_raise_size, bb)`.
2. **An all-in for less than a full raise does not reopen betting** for players who already acted on that street. They
   may only call or fold. `legal.raise` must be `null` for them.
3. A player can go all-in at any time with `raise` to `max_to` (or `call` if the stack ≤ the call amount).
4. Side pots are built standard-wise. Odd chips go to the first winner clockwise from the button.
5. Preflop action starts left of the BB. Postflop it starts with the first active seat left of the button.
6. Fold when check is free: allowed. The server converts it to a check and reports `"type":"check"`, or rejects it
   with `error.code="fold_when_check_free"`. Pick one and document which; converting it to a check is preferred.
7. Timeout: check if free, else fold. The event carries `timeout: true`.
8. `stack_mode:"reset"`: every hand, every seated player starts at `starting_stack`. **Required for evaluation.**
   The bot was trained for 100 bb; with `"carry"` it caps its own risk at 150 bb per hand but plays
   less well.
9. Disconnects: the seat keeps playing with the timeout policy; the player can reconnect and resume (§4.3, `table_snapshot`).
10. Rake: none in evaluation matches.

---

## 7. Dealing, seeds and duplicate

* Deck for deal `i` of a match: `rng = ChaCha20(seed = HMAC_SHA256(match.seed, "deal:" + i))`, then a Fisher-Yates
  shuffle of the 52 cards in order `2c 2d 2h 2s 3c … As`. Hole cards are dealt one at a time clockwise
  starting left of the button (2 rounds), then burn+3 flop, burn+1 turn, burn+1 river. Any fixed documented
  procedure works; it just has to be **deterministic and published** so hands can be re-simulated.
* `deck_commitment = sha256(deck.join("") + salt)` at `hand_start`, revealed at `hand_end`.
* **Duplicate**: deal `i` is played `rotations=6` times. In rotation `r`, participant `p` sits in seat `(p + r) mod 6`.
  The button is at the same physical seat for all rotations of a deal, so the cards fall on the same seats and only
  the players change. Rotations of one deal should run on **different tables in parallel or far apart in time**
  (bots don't remember, humans do).
* Reproducibility: `POST /v1/matches` with the same `seed` and participants must produce identical decks.

---

## 8. Hand history (REST, export, and what the evaluation pipeline reads)

```ts
type HandHistory = {
  hand_id: string; hand_no: number; match_id: string | null; table_id: string;
  deal_no: number | null; rotation: number | null;
  started_at: string; ended_at: string;
  config: { small_blind: number; big_blind: number; ante: number; starting_stack: number; stack_mode: string };
  button_seat: number; sb_seat: number; bb_seat: number;
  players: Array<{ seat: number; player_id: string; name: string; kind: "bot"|"human"; stack_start: number }>;
  hole_cards: Record<number, [string, string]>;     // all
  board: string[];                                  // as dealt (≤5)
  runout: string[];                                 // the full 5-card board as it WOULD have been dealt
  deck: string[]; deck_salt: string;
  actions: ActionEvent[];                           // every event in seq order (incl. blinds/timeouts)
  decision_times_ms: Record<string, number>;        // action_id -> think time
  pots: HandEnd["pots"];
  results: Array<{ seat: number; player_id: string; net: number; stack_end: number }>;
  ended_by: "showdown" | "fold";
  anomalies: string[];                              // server-side issues (should be [])
};
```

---

## 9. Standings (`GET /v1/matches/{id}`)

```ts
type Standing = {
  player_id: string; name: string;
  hands: number;
  net_chips: number;
  bb_per_100: number;             // raw
  mbb_per_hand: number;           // raw
  ci95_bb_per_100: number;        // from per-hand sd
  duplicate_bb_per_100?: number;  // averaged over completed deals (all 6 rotations)
};
```

The server only needs raw stats. AIVAT is computed on our side from the hand histories plus the bot's own
policy probabilities.

---

## 10. Error codes

| code | http | meaning |
|---|---|---|
| `unauthorized` | 401 | bad/missing token |
| `not_your_turn` | 409 | `act` without a pending `your_turn` |
| `stale_decision` | 409 | `decision_id` already answered or expired |
| `illegal_action` | 422 | action type not in `legal` |
| `bad_amount` | 422 | `to` outside `[min_to, max_to]` (details: `min_to`, `max_to`) |
| `fold_when_check_free` | 422 | only if the server rejects it (§6.6) |
| `table_full` / `seat_taken` | 409 | |
| `not_seated` | 409 | |
| `match_not_running` | 409 | |
| `rate_limited` | 429 | |
| `internal` | 500 | |

On `illegal_action` / `bad_amount` the server re-sends `your_turn` with the same `decision_id` if the deadline has not
passed. Otherwise the timeout policy applies.

---

## 11. Optional webhook mode (stateless bots)

Instead of the WebSocket, a player registers `{"webhook_url": "https://bot/act", "secret": "..."}`. The server `POST`s
the `your_turn` envelope (§5.3), signed with `X-Signature: sha256=HMAC(secret, body)`. The bot replies `200`
with the `act` data, which must arrive before `deadline_ms`. `hand_start`/`action`/`street`/`hand_end` are POSTed
fire-and-forget to `/events` on the same base URL, if configured.

---

## 12. Conformance tests (the server should pass these before we play for real)

Publish a test table, `POST /v1/tables` with `"mode":"sandbox"`, where the server deals **scripted** decks and accepts
scripted actions, plus these vectors:

1. Blinds and first action: 6 seats, button 3, so SB 4, BB 5, first to act 0. Heads-up: button = SB, acts first preflop.
2. Min-raise chain: open to 300 → min re-raise to 500 → re-raise to 1200 → min next = 1900.
3. Short all-in does not reopen: A bets 1000, B all-in to 1300 (< full raise), A may only call or fold.
4. Side pots: 3 players all-in at 2000 / 5000 / 10000 → main 6000, side 6000, side 5000 with correct eligibility.
5. Odd chip: split pot of 1001 between 2 winners → 501 to the first winner left of the button.
6. Timeout: no answer → check if free else fold, `timeout:true` in the event.
7. Reconnect mid-hand: drop the socket during `your_turn`, reconnect with `hello{resume}` → `table_snapshot` + the same
   `your_turn` (same `decision_id`) if the deadline has not passed.
8. Duplicate: the same `seed` gives the same decks. Rotation `r` puts participant `p` in seat `(p+r) mod 6`.
9. `sum(results.net) == 0` every hand (no rake) and `stack_end == stack_start + net`.

---

## 12b. Official rules (ambiguities resolved)

1. **House bots** have player ids `house:<name>`: `house:random`, `house:calling_station`, `house:tight_aggressive`, `house:nit`, `house:lag`, `house:maniac`, and `house:sharingan-1` (only when the server has a blueprint configured, otherwise a clear `400`). They can be seated via REST (`POST .../seats/{n}` with `{"house":"tight_aggressive"}`, or `POST .../fill`) and used in match `participants`. `ladder` = random, calling_station, 2x tight_aggressive, sharingan-1; `POST /v1/matches` takes `"field":"ladder"` to append it to your `participants`.
2. `hand_id` and `seq` appear both in the envelope and in `your_turn.data`. `action`, `street` and `hand_end` share one gap-free per-hand `seq` (blinds are 1 and 2); `your_turn` carries the `seq` the *next* event will take and does not consume it, so a client that ignores `your_turn` sees no gaps. `hand_start` and `hole_cards` are unsequenced.
3. Match results: `standings` is a **list** of objects with `player_id`, and each has `mbb_per_game` and `ci95_mbb` (the luck-corrected score: AIVAT if available, else duplicate, else raw) plus `bb_per_100`, `ci95_bb_per_100`, `duplicate_bb_per_100`, `net_chips`, `hands`, `method`.
4. `hello{resume}` returns an authoritative `table_snapshot` (no event replay), then re-sends any pending `your_turn` with the same `decision_id`.
5. After `illegal_action` / `bad_amount` the server re-sends `your_turn` with the **same** `decision_id`; the client may retry until the deadline.
6. Fold when check is free is converted to a check (no error); the event reports `"type":"check"`.
7. `hand_end.hole_cards` is an object keyed by the seat number as a JSON string.
8. Webhook mode: `POST /v1/players` accepts `webhook_url` and `secret` (if the server enables webhooks). The server POSTs the `your_turn` envelope with header `X-Sharingan-Signature: sha256=HMAC_SHA256(secret, body)`; the bot replies `{hand_id, decision_id, type, to?}`.
9. `TableConfig.decision_time_ms` defaults to 5000. `mode` is `"sandbox"` (scripted decks: `sandbox.decks`, `sandbox.button_seat`, `sandbox.stacks`), `"ranked"` or `"benchmark"` (item 12).
10. Duplicate schedule: deal `i` has button seat `i mod 6`; deals are played in blocks (default 12 deals) rotation-major, so the six rotations of a deal are far apart. Hand histories of a running match are visible only to its owner. Optional match field `"reveal":"match_end"` withholds other players' cards in `hand_end` until the match ends.

11. **`table_seq` is gap-free per recipient.** Each connection's player gets its own counter, incremented once per message delivered to that player at that table (starting at 1 for the first message after sitting down). Events private to another seat (their `hole_cards`, `your_turn`) never consume your numbers. `table_snapshot` carries the recipient's current value in both the envelope and `data.table_seq`; the next message is +1. A re-sent `your_turn` (resume, illegal-action retry) repeats its original `table_seq`. The copy stored in `/v1/hands/{id}/events` carries the table-wide counter instead.
12. **`mode: "benchmark"`** (tables and matches): the fast path. No pause between hands, and each player receives only `your_turn` (full history inside, hole cards inside) and `hand_end` (other players' cards redacted unless shown at showdown when the match uses `reveal: "match_end"`, which benchmark runs do). No `hand_start`, `hole_cards`, `action` or `street` events. Hand histories are still stored (written in batches, so reads see them within milliseconds). Duplicate matches accept `"lanes": N`: the deals are cut into blocks and lane `L` plays blocks `L, L+N, ...` on its own table, in parallel; the same bot sits at every lane table, so bots must key state by `table_id`/`hand_id`. Results do not depend on the lane count. Measured with the Python SDK on a 128-core box (release build, bot = check/call, 5 house bots): 5 hands/s before this wave (a TCP Nagle/delayed-ACK stall plus `table_seq`/`seq` gap resync storms), 115-150 hands/s in normal mode and 203 hands/s in benchmark mode after, 552 hands/s with `lanes=4` and 688 with `lanes=8`; a server-hosted bot runs ~4,800 hands/s end to end including the AIVAT pass.
13. **Benchmark tracks** are versioned, frozen panels, run as duplicate matches with a fixed seed (`bench:<track>:v<version>`), so every bot sees the same deals and scores are paired. `GET /v1/benchmarks` lists them (panel, seed, `available`, blueprint fingerprint).
    * `ladder-v1`: random, calling_station, 2x tight_aggressive, sharingan-1. `sharingan-1`: 5x sharingan-1. Both need `ARENA_BLUEPRINT` (else `400`).
    * `POST /v1/benchmarks/{track}/runs` `{player_id?, hands?, lanes?}` (your own bot; admins may run any bot, including `house:*`) starts the run at once and returns the run. A remote bot must already be connected to `/v1/play` (`409 not_connected` otherwise). `hands` defaults to **20,000** and is rounded up to whole deals (a multiple of 6, so 20,004). `GET /v1/benchmarks/{track}/runs/{run_id}` returns the run with live progress (`hands_done`, `hands_total`, `running_mbb_per_game`, `running_ci95_mbb`); when finished, `mbb_per_game` and `ci95_mbb` (AIVAT, the server-side luck correction); `POST .../abort` stops it (the partial result is kept). Runs are stored per bot and track; the latest finished run counts.
    * **`verified`** is true only when the server itself ran the bot (house/hosted bots). A remote bot's code runs on its owner's machine and the seed is public, so its runs are ranked but flagged `verified: false`.
14. **WebSocket `benchmark_progress`** is pushed to the bot's own connection about every 1% of the run (at least every 50 hands) and once at the end: `{run_id, track, match_id, status, hands_done, hands_total, mbb_per_game, ci95_mbb, method, verified?}`. `match_update` carries the same running standings for plain matches.
15. **Leaderboard** `GET /v1/leaderboard[?track=&min_hands=&include_house=]` returns `{rows}`. Each row has the blended score (`score_mbb_per_hand`, `ci95_mbb_per_hand`, null below `min_hands`), `vs_ladder_v1` and `vs_sharingan_1` (`{mbb, ci95_mbb, hands}` or null), `verified`, and `tracks` (the same per track plus `run_id`, `method`, per-track `verified`). `?track=` keeps only bots with a run on that track and ranks by it. A house bot may fill several seats of one match; each slot is scored separately (`slot` in the standing) and only the first occurrence is stored per player.

---

## 13. What our bot does with this, and what still has to be built on our side

**Already true.** The bot's NLHE engine (`engine/nlhe`) implements exactly the rules in §6: `to` semantics,
`min_raise_to = current_bet + max(last full raise, bb)`, a short all-in does not reopen betting, the odd chip goes
left of the button, and a fold when check is free is treated as a check. The Open Poker client
(`arena/openpoker`) already shows the pattern it uses:
* it rebuilds its own engine from the server's events and cross-checks stacks and pot on every event;
* it runs the blueprint or turn/river search with a hard 5 s budget and a blueprint fallback;
* it clamps each action to the legal set;
* it applies a stack guard and a reset above 150 bb, and writes a full decision trace.

**To build once this API exists** (about a day):
1. **Protocol client** `arena/tableapi`, a sibling of `arena/openpoker`: WebSocket client for §4/§5, `decision_id`
   handling, seq-gap detection with `hello{resume}` resync, reconnect, and mapping `your_turn` to the existing
   session/engine/policy stack.
2. **Live AIVAT**: Open Poker hides folded cards, so the live analyzer only does an all-in luck adjustment today.
   With `hand_end.hole_cards` and `runout` (§5.4, §8), compute full AIVAT per hand from our logged policy
   probabilities. This is the same estimator as the offline `bench::aivat`.
3. **Duplicate standings**: aggregate results per `deal_no` across the 6 rotations (§7).
4. **Conformance run**: play the §12 vectors against the sandbox table before any real match.

**What we need from you:** the `wss://` URL, one bot token per seat we run, and a `match_id` with
`stack_mode:"reset"`, `starting_stack:10000`, `sb:50`, `bb:100` and `decision_time_ms ≥ 3000`.
