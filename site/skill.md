---
name: poker-sharingan
description: Build a six-player no-limit hold'em poker bot, score it with luck-corrected evals, and put it on the Poker Sharingan leaderboard (sharingan.poker). Use when asked to make, test, evaluate, benchmark or submit a poker bot, or to post poker results to the leaderboard.
---

# Poker Sharingan: build a bot, score it, get on the leaderboard

Arena: `https://arena.sharingan.poker` · Leaderboard: https://sharingan.poker/leaderboard · Free, no account.
Six-max no-limit hold'em, blinds 50/100, 100 bb stacks. Scores are **mbb/game** (1000 mbb = 1 big blind) with a 95% confidence interval.

## 1. Install (Python 3.9+)

```bash
python3 -m venv .venv && . .venv/bin/activate
pip install "git+https://github.com/markokraemer/poker-sharingan#subdirectory=sdk/python"
export SHARINGAN_SERVER=https://arena.sharingan.poker
```

## 2. Write the bot

One file, one function. `s` is the decision you have to make.

```python
# mybot.py
from sharingan import Action

def act(s):
    if s.legal.raise_ and s.hole_cards[0][0] in "AK":   # cards are strings: "As", "Td", "9h"
        return Action.raise_to(3 * s.big_blind)          # amounts are the total "to", never "by"
    if s.legal.check:
        return Action.check()
    return Action.call() if s.pot_odds < 0.25 else Action.fold()
```

| `s.` | meaning |
|---|---|
| `hole_cards`, `board` | your two cards, the 0/3/4/5 board cards |
| `street` | `preflop` · `flop` · `turn` · `river` |
| `legal.check`, `legal.call`, `legal.raise_` | what is allowed; `legal.raise_.min_to` / `.max_to` (or `s.min_raise_to` / `s.max_raise_to`) |
| `to_call`, `pot`, `big_blind` | chips (ints) |
| `pot_odds` | `to_call / (pot + to_call)`: the share of the final pot you pay to call |
| `seat`, `button_seat`, `players`, `me` | `players[i]` has `seat`, `name`, `stack`, `street_contrib`, `hand_contrib`, `status` (active, folded, all_in) |
| `history` | every action this hand: `seat`, `type`, `to`, `street` |

Actions: `Action.fold()`, `Action.check()`, `Action.call()`, `Action.raise_to(n)`. Illegal choices are fixed for you (fold when check is free becomes check; amounts are clamped). Answer within 5 s or the server plays check/fold. A `Bot` subclass with `act(self, s)` also works, if the bot keeps state.

## 3. Try it (seconds, not ranked)

```bash
sharingan play mybot.py --vs sharingan-1 --hands 12     # prints every hand
```

## 4. Submit: verified score on the leaderboard

```bash
sharingan submit mybot.py                               # track ladder-v1, 20,000 hands
sharingan submit mybot.py --track sharingan-1           # the hard track: 5 copies of SHARINGAN-1
```

* The first run registers the bot (named after the file) and prints its token `sk_...` on stderr. Save it right away and `export SHARINGAN_TOKEN=sk_...` before every later run (`submit`, `benchmark`, `eval` all read it), so new scores land on the **same leaderboard row**. Without it each run can become a new bot.
* The result is the line `mybot: +132.4 +- 41.0 mbb/game over 20004 hands on ladder-v1 [VERIFIED, hosted job ...]`, then `leaderboard: <link>`. A 20,000-hand job takes under a minute.
* Our machines run the bot in a sandbox: Python, **no network**, 4 CPU, 12 GB. Every run gets fresh secret deals, each replayed from every seat, so luck cancels out.
* More files or dependencies: put them in a directory, then `sharingan submit botdir/ --entry bot.py --requirements requirements.txt`.
* Limits: `--hands` 600 to 200,000 (rounded up to a multiple of 6), 1M hands per bot per day, 2 jobs at a time. Stop a job: `curl -X POST $SHARINGAN_SERVER/v1/jobs/JOB_ID/abort -H "authorization: Bearer $SHARINGAN_TOKEN"`.

## 5. Bots that need the network (LLM bots, remote models)

```bash
sharingan benchmark mybot.py --track ladder-v1
```

The bot runs on your machine and plays over a WebSocket. Same score and leaderboard, marked **unverified** (no ✓), because we can't see the code that decided. Use fewer hands for slow bots (`--hands 1200`). `from sharingan.bots import LLMBot` wraps any `complete(prompt) -> str`.

## 6. Read results

```bash
curl -s "https://arena.sharingan.poker/v1/leaderboard?track=ladder-v1"     # JSON, ranked by that track
```

Each row: `rank`, `name`, `player_id`, `verified`, and per track `tracks.<track>.mbb_per_game` / `.ci95_mbb` / `.hands`. Without `?track=` rows are ranked like the site: sharingan-1, then ladder-v1.

## 7. Ask SHARINGAN-1 directly (inference)

Use our bot inside your own code, sim or analysis: send a spot, get its action and probabilities. Any arena token works (10 decisions/s).

```bash
curl -s https://arena.sharingan.poker/v1/act -H "authorization: Bearer $SHARINGAN_TOKEN" -H 'content-type: application/json' -d '{
  "small_blind": 50, "big_blind": 100, "button_seat": 3, "hero_seat": 0, "hole_cards": ["Ah", "Kd"], "board": [],
  "players": [{"seat":0,"stack_start":10000},{"seat":1,"stack_start":10000},{"seat":2,"stack_start":10000},
              {"seat":3,"stack_start":10000},{"seat":4,"stack_start":10000},{"seat":5,"stack_start":10000}],
  "history": []}'
# {"action": {"type": "raise", "to": 250, ...}, "probs": [...], "source": "blueprint", ...}
```

`history` is every action so far (`{"seat": 1, "type": "raise", "to": 300}`, types `fold` `check` `call` `bet` `raise` `all_in`), `board` holds the cards of the current street, `hero_seat` must be the player to act. Full format: https://github.com/markokraemer/poker-sharingan/blob/main/sharingan-1/serve/README.md

## Report back to the user

The score line (mbb/game ± CI, hands, track, VERIFIED or not), the leaderboard link, and where the token is saved. If the interval contains 0, say the bot is not distinguishable from break-even yet; more hands narrow it.

Other languages, the raw protocol and the hand-history format: https://github.com/markokraemer/poker-sharingan/blob/main/docs/TABLE_API.md
