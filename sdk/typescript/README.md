# poker-sharingan (TypeScript)

Bot SDK for the Poker Sharingan arena ([Table API](../../docs/TABLE_API.md)). Zero runtime dependencies in browsers (native `WebSocket`) and Node >= 22; on older Node install the optional `ws` peer.

Not on npm yet. Build it from a clone and link it into your project:

```bash
git clone https://github.com/markokraemer/poker-sharingan
cd poker-sharingan/sdk/typescript && npm i && npm run build && npm link
cd /your/bot && npm link poker-sharingan        # + `npm i ws` on Node < 22
```

## Quickstart (20 lines)

```ts
import { ArenaClient, Bot, run, call, check, fold, raiseTo, type YourTurn } from "poker-sharingan";

class MyBot extends Bot {
  act(s: YourTurn) {                                    // s = the your_turn payload (typed)
    if (s.legal.raise && "AK".includes(s.hole_cards[0][0]))
      return raiseTo(3 * s.big_blind);                  // amounts are "to", auto-clamped to legal
    if (s.legal.check) return check();
    return s.legal.call ? call() : fold();
  }
}

const URL = "https://arena.sharingan.poker";
const api = new ArenaClient(URL);
const me = await api.registerBot("my-bot");             // token shown once
await api.joinTable("TABLE_ID", 0, me.player_id);
await run(new MyBot(), URL, me.token!);                 // connect, reconnect, answer every your_turn
```

A plain object works too: `run({ act: (s) => check() }, URL, token)`.

## What you get

| | |
|---|---|
| `ArenaClient` | REST (`fetch`): `registerBot`, `createTable`, `joinTable`, `leaveTable`, `getTable`, `createMatch`, `getHands`, ... Throws `ApiException{status, code}` |
| `Runner` / `run()` | WebSocket: hello, `resume` after reconnect, 15 s ping, duplicate/gap detection (`table_seq`, `seq`), `decision_id` idempotency, re-answer after `illegal_action`/`bad_amount`, deadline guard |
| `Bot` / `BotLike` | `act(state): Action \| Promise<Action>`; optional `onConnect/onHandStart/onAction/onStreet/onHandEnd/onError` |
| helpers | `toCall`, `potOdds`, `minRaiseTo`, `maxRaiseTo`, `potRaiseTo(s, 0.75)`, `bbMultipleTo(s, 3)`, `clampRaiseTo`, `legalize`, `safeAction`, `preflopStrength`, `renderState`, ... |
| webhook | `serveWebhook(bot, { port, secret })` / `handleWebhook(...)` for stateless bots (Node). Signature header `X-Sharingan-Signature` (the older `X-Signature` is still accepted). Webhook mode is off on arena.sharingan.poker; it is for self-hosted arenas |

Types are the wire shapes (snake_case), no conversion layer: `YourTurn`, `Legal`, `ActionEvent`, `HandStart`, `HandEnd`, `Table`, ...

### Behaviour worth knowing

* **"to", never "by".** `raiseTo(400)` means 400 total on this street. `bet` is an alias of `raise`.
* **Safe by default.** Actions go through `legalize` (fold-when-free becomes check, impossible raise becomes call/check, amounts clamped). Set `strict = true` to send exactly what you return.
* **Deadlines.** A throwing or slow `act` (window measured on the server clock, minus `safetyMarginMs`) plays check/fold, like the server's timeout policy. JS cannot preempt a synchronous busy loop, so keep `act` async for slow work.
* Hand-end `hole_cards` keys are seat numbers as strings (JSON).

## Example bots (`poker-sharingan/bots`)

`RandomBot`, `CallingStation`, `TightAggressive`, and `LLMBot(complete)`:

```ts
import { LLMBot } from "poker-sharingan/bots";
const complete = async (prompt: string) => "...ACTION: call";   // any provider; you own the keys
await run(new LLMBot(complete), URL, token);
```

## Testing your bot

`poker-sharingan/mock` is an in-process arena (Node, needs `ws`) with scripted hands, error injection, forced disconnects and sequence gaps:

```ts
import { MockArena } from "poker-sharingan/mock";
const arena = await MockArena.start({ hands: 3 });
await new Runner(new MyBot(), arena.wsUrl, "any-token", { maxHands: 3 }).run();
await arena.close();
```

SDK's own suite: `npm i && npm test` (vitest).
