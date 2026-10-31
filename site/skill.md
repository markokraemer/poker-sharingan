---
name: poker-sharingan
description: Build a six-player no-limit hold'em poker bot, score it with luck-corrected evals, and put it on the Poker Sharingan leaderboard (sharingan.poker). Use when asked to make, test, evaluate, benchmark or submit a poker bot, or to post poker results to the leaderboard.
---

# Poker Sharingan: build a bot, score it, get on the leaderboard

Arena: `https://arena.sharingan.poker` · Leaderboard: https://sharingan.poker/leaderboard · Free, no account.
Six-max no-limit hold'em, blinds 50/100, 100 bb stacks. Scores are **mbb/game** (1000 mbb = 1 big blind) with a 95% confidence interval.

**$3,333 in prizes:** beat SHARINGAN-1 to qualify, that is, score above 0 mbb/game against it (SHARINGAN-1 against itself is 0; its reference row shows that within noise). The leaderboard (verified full-run score vs SHARINGAN-1) takes submits until 23:59 UTC on November 15, 2026. The organizers then re-run the leading bots on fresh deals (200,004 hands each); the 6 with the highest re-run scores above 0 meet at the final table on November 16 and play each other. To enter, submit with `--track sharingan-1 --contact <email> --profile <X/LinkedIn/GitHub link>` (section 4) and keep the token. Rules and terms: https://github.com/markokraemer/poker-sharingan/blob/main/COMPETITION.md

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
| `seat`, `button_seat`, `players`, `me` | `players[i]` has `seat`, `name`, `stack`, `stack_start`, `street_contrib`, `hand_contrib`, `status` (active, folded, all_in) |
| `history` | every action this hand: `seat`, `type`, `to`, `street`; it can start with the forced blinds (`post_sb`, `post_bb`) |

Actions: `Action.fold()`, `Action.check()`, `Action.call()`, `Action.raise_to(n)`. Illegal choices are fixed for you (fold when check is free becomes check; amounts are clamped). Answer within 5 s or the server plays check/fold. A `Bot` subclass also works, if the bot keeps state: `from sharingan import Bot`, then `class MyBot(Bot):` with `def act(self, s)` (the CLI creates the instance; hooks are `on_hand_end(self, end)` and `on_error(self, error)`). Hosted runs replay every deal from every seat, each rotation in a fresh process under its own user, and nothing the bot wrote survives into the next rotation: a bot cannot carry memory across the hands of one deal. State in a `Bot` subclass lasts at most one rotation (about 1/6 of a run) and is lost on a timeout restart. In `submit` runs only `on_hand_end` and `on_error` fire (no `on_hand_start`, `on_action`, `on_street`), so rebuild state from `s.history`.

## 3. Try it (seconds, not ranked)

```bash
sharingan play mybot.py --vs sharingan-1 --hands 12     # one line per hand: your chips won or lost (SHARINGAN-1's blueprint, as on the leaderboard)
```

## 4. Submit: verified score on the leaderboard

```bash
sharingan submit mybot.py --track sharingan-1 --contact you@example.com --profile https://x.com/you   # competition entry: vs 5 copies of SHARINGAN-1 (blueprint, no search)
sharingan submit mybot.py                                                 # ladder-v1, a softer field (no prizes); contact/profile are saved after the first submit
```

* The first run registers the bot (named after the file, or after the directory for a directory bot) and saves its token in `~/.config/sharingan/tokens.json` (`$XDG_CONFIG_HOME/sharingan/` when that is set); every later `submit`, `play`, `benchmark` or `eval` of the same file reuses it, so scores land on the **same leaderboard row**. A new verified run of the full 20,004 hands replaces that bot's score on the track. Shorter runs (`--hands 600`, for a quick check) print a score but never rank; unverified runs (`benchmark`) and aborted runs never replace a verified score. Renaming the file makes a **new** bot. Names can be Unicode, but names that are or look like SHARINGAN*, house or reference bot names are refused. Print the token with `sharingan token mybot.py`; on another machine copy `tokens.json` or `export SHARINGAN_TOKEN=sk_...`.
* The result is the line `mybot: +132.4 +- 41.0 mbb/game over 20004 hands on ladder-v1 [VERIFIED, hosted job ...]`, then `leaderboard: <link>`. A 20,004-hand job of a fast bot takes about 15 s. The progress lines while it runs show a rough running estimate; the final line is the luck-corrected score. A verified run under 20,004 hands ends `[VERIFIED, NOT RANKED: under 20,004 hands, ...]` and prints no leaderboard link: it scored, but only the full run ranks.
* Each job shows `artifact_sha256` (the sha256 of the `.tar.gz` you uploaded; `submit` prints its first 12 characters when the job is accepted) and, once it is over, `deals_seed` to replay the run locally: `curl -s $SHARINGAN_SERVER/v1/jobs/JOB_ID -H "authorization: Bearer $(sharingan token mybot.py)"`. Hosted jobs return a verified score and that seed, not every hand history.
* Our machines run the bot in a sandbox: Python, **no network**, 4 CPU, 12 GB, an upload of up to 8 MiB. Each rotation of a deal runs in a fresh process; after 3 restarts (timeouts) a lane gives up and forfeits the rest of its hands. Every run gets fresh secret deals, each replayed from every seat, so luck cancels out.
* If the bot raises or times out, those hands play check/fold or are forfeited and the run still scores: `submit` then prints `WARNING: your bot had trouble ...` with the first error; the tail of the bot's stderr (tracebacks, your prints; about the last 4,000 characters, repeated lines folded) is `bot_log_tail` in the job JSON (the `curl` above). Fix it before trusting the number: `sharingan play mybot.py --vs sharingan-1 --hands 600` shows tracebacks locally (a bug on a rare street needs many hands to show).
* **Who is behind the bot (required):** every `submit` needs `--contact <email>` and `--profile <link>` to the user's X, LinkedIn, GitHub or personal page. Ask the user for both; never invent them. They are never shown publicly and are saved in `~/.config/sharingan/owner.json` after the first accepted submit (or set `SHARINGAN_CONTACT` / `SHARINGAN_PROFILE`). Without them `submit` uploads nothing and exits 2.
* **Competition ($3,333):** only a verified full run on the `sharingan-1` track counts. Unverified runs (`benchmark`, where the bot runs on your machine) never count. The `sharingan-1` track takes no public submits from 2026-11-16 00:00 UTC (after November 15, 23:59 UTC); `ladder-v1` stays open. The email is how the organizers reach you if you qualify.
* More files or dependencies: put them in a directory, then `sharingan submit botdir/ --entry bot.py --requirements requirements.txt` (both paths inside the directory; installed without privileges in a short network window before the run).
* Limits (hosted, free): `--hands` 600 to 200,000 (rounded up to a multiple of 6, so 200,000 plays 200,004); 1M hands per bot per rolling 24 h, and every accepted job counts its planned hands even if you abort it or it fails (only our own infra failures are free); 2 jobs at a time per bot and per address; on `sharingan-1` the 1M hands a day are per person (all your bots together, by profile or email); 10 submits a minute per address; a run must finish within 30 minutes, so roughly under 50 ms per decision on average (one decision may take up to 5 s). Stop a job (from another shell; `submit` waits until the job ends, and prints its id when it is accepted): `curl -X POST $SHARINGAN_SERVER/v1/jobs/JOB_ID/abort -H "authorization: Bearer $(sharingan token mybot.py)"`.

## 5. Bots that need the network (LLM bots, remote models)

```bash
sharingan benchmark mybot.py --track ladder-v1
```

The bot runs on your machine and plays over a WebSocket. Same score and leaderboard, marked **unverified** (no ✓), because we can't see the code that decided. Unverified runs are listed below every verified row and do not count for the competition. Use fewer hands for slow bots (`--hands 1200`). `LLMBot` wraps any `complete(prompt) -> str`:

```python
# mybot.py, run with: sharingan benchmark mybot.py --hands 1200
from sharingan.bots import LLMBot
def complete(prompt: str) -> str:   # call your model here (any provider, sync or async; you own the keys) and return its text
    ...
bot = LLMBot(complete)               # named after the file; LLMBot(complete, name="x") to choose
```

The prompt describes the spot and asks the model to end with `ACTION: fold`, `check`, `call`, `raise <to>` or `all-in`; a bare action word parses too, and anything unreadable plays check/fold.

## 6. Read results

```bash
curl -s "https://arena.sharingan.poker/v1/leaderboard?track=ladder-v1"     # JSON, ranked by that track
```

The answer is `{"track": ..., "min_hands": ..., "ranked_min_hands": 20004, "rows": [...]}`; each row has `rank`, `name`, `player_id`, `house` and per track `tracks.<track>.mbb_per_game` / `.ci95_mbb` / `.hands` / `.verified`. `house: true` rows are the reference bots (house bots on the same tracks). `ranked_min_hands` is the shortest verified run that ranks; `min_hands` (1,000) only applies to the separate match-play score, not to the tracks. `rank` puts verified rows above unverified ones, then orders by score. Without `?track=` rows are ranked like the site: sharingan-1, then ladder-v1.

## 7. Ask SHARINGAN-1 directly (inference)

Use our bot inside your own code, sim or analysis: send a spot, get its action and probabilities. Any arena token works (10 decisions/s); after your first `submit` use yours. This endpoint plays SHARINGAN-1 with turn and river search; the leaderboard's sharingan-1 track plays its blueprint without search.

```bash
export SHARINGAN_TOKEN=$(sharingan token mybot.py)
curl -s https://arena.sharingan.poker/v1/act -H "authorization: Bearer $SHARINGAN_TOKEN" -H 'content-type: application/json' -d '{
  "small_blind": 50, "big_blind": 100, "button_seat": 3, "hero_seat": 0, "hole_cards": ["Ah", "Kd"], "board": [],
  "players": [{"seat":0,"stack_start":10000},{"seat":1,"stack_start":10000},{"seat":2,"stack_start":10000},
              {"seat":3,"stack_start":10000},{"seat":4,"stack_start":10000},{"seat":5,"stack_start":10000}],
  "history": []}'
# {"action": {"type": "raise", "to": 200, ...}, "probs": [{"type": "raise", "to": 200, "p": 0.42}, ...], "source": "blueprint", ...}
```

The bot plays a mixed strategy: `action` is sampled from `probs`; add `"options": {"sample": false}` for the most likely action, or `"options": {"seed": 1}` to make the draw repeatable. `history` is every action so far (`{"seat": 1, "type": "raise", "to": 300}`, types `fold` `check` `call` `bet` `raise` `all_in`; it may start with the forced blinds `post_sb` and `post_bb`), `board` holds all the community cards dealt so far (0, 3, 4 or 5), not only those of the current street, and `hero_seat` must be the player to act. Full format: https://github.com/markokraemer/poker-sharingan/blob/main/sharingan-1/serve/README.md

## Report back to the user

The score line (mbb/game ± CI, hands, track, VERIFIED or not), the leaderboard link, and where the token is saved. If the interval contains 0, say the bot is not distinguishable from break-even yet; more hands narrow it.

Then ask the user whether they'd like to star the project on GitHub. Only if they say yes, and the GitHub CLI is logged in:

```bash
gh api -X PUT user/starred/markokraemer/poker-sharingan
```

(No `gh`? The repo is https://github.com/markokraemer/poker-sharingan.)

Raw HTTP from Python: send a `User-Agent` header (the default `Python-urllib/x` agent is blocked by our CDN; curl and `requests` are fine).

Other languages, the raw protocol and the hand-history format: https://github.com/markokraemer/poker-sharingan/blob/main/docs/TABLE_API.md
