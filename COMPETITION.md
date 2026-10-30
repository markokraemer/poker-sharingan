# Poker Sharingan Competition #1

**$5,000 in prizes. Build a six-player no-limit hold'em bot, beat SHARINGAN-1, climb the leaderboard. Ends November 15, 2026, 23:59 UTC.**

Read the [Terms](#terms) below. This is an informal community competition: prizes are discretionary and non-binding.

## Enter

```bash
pip install "git+https://github.com/markokraemer/poker-sharingan#subdirectory=sdk/python"
sharingan play mybot.py --vs sharingan-1 --hands 12      # try it, not ranked
sharingan submit mybot.py --track sharingan-1 --contact you@example.com   # verified score on the leaderboard
```

* No account needed. The first `submit` registers the bot, named after the file (`deepstack.py` plays as `deepstack`), and saves its token in `~/.config/sharingan/tokens.json`. Keep the token: it is how you prove the bot is yours.
* `--contact` takes an email or an X handle (`@you`). It is how we reach you if you win and is never shown publicly. Add it to at least one submit of the bot.
* How to write `mybot.py`: [sharingan.poker/docs](https://sharingan.poker/docs/). With a coding agent, one prompt: *"Read https://sharingan.poker/skill.md, then build a poker bot and get it on the leaderboard."*
* Only **hosted, verified** runs count (`sharingan submit`): our machines run the bot in a sandbox (Python, no network, 4 CPU, 12 GB). `benchmark` runs (your machine, unverified) do not count.

## How bots are ranked

1. **The leaderboard.** Each `submit` plays 20,004 hands against five copies of SHARINGAN-1 on fresh secret deals. Every deal is replayed from every seat and the luck is subtracted (duplicate + AIVAT), so the score is mbb per game with a 95% interval. A new run replaces the bot's previous score. The live order is the leaderboard's default sort (score vs SHARINGAN-1): https://sharingan.poker/leaderboard/
2. **The final run decides.** Submissions after the deadline do not count. After it, the organizers take the top submitted bots on the leaderboard as of the deadline (about the top 10) and re-run each bot's last verified submission before the deadline on new secret deals over a much larger sample (about 200,000 hands each). The final run, not the live leaderboard, decides the ranking. This keeps luck and lucky resubmits from deciding a prize.
3. **Who can place.** Reference bots (marked REFERENCE, including SHARINGAN-1) do not place. Bots that are, or mostly reproduce, SHARINGAN-1 or another entrant's bot may be excluded.

## Prizes

* **$5,000 in total** for the top 3 bots, split as the organizers decide.
* Winners are announced after the final run, on [sharingan.poker](https://sharingan.poker) and in this repository.
* We contact winners through the `--contact` they gave. No contact on file: open a [GitHub issue](https://github.com/markokraemer/poker-sharingan/issues) within 14 days of the announcement (**never post the token**). We then ask for proof of ownership of the bot (its token) and the details needed to pay.

## Sponsors

We want to grow the prize pool. If you want to add to it, open a [GitHub issue](https://github.com/markokraemer/poker-sharingan/issues) or contact the organizers.

## Terms

By submitting a bot you accept these terms.

1. **Discretionary, non-binding.** This is an informal community competition run for fun and research. It is not a contract, an offer, a lottery or a game of chance, and nothing in it creates a legal obligation for the organizers. Prizes are discretionary awards, not entitlements.
2. **No guarantee of payment.** The organizers may award, reduce, split, postpone, withhold or not pay any prize, in whole or in part, for any reason, including a lack of sufficient entries or quality, suspected abuse, eligibility, legal, tax or payment constraints.
3. **Changes and cancellation.** The organizers may change these rules, the ranking method, the tracks, the deadline, the prize amounts or these terms, or suspend or cancel the competition, at any time and without notice. The current version of this file is the one that applies.
4. **Final decisions.** The organizers decide all questions about entries, rankings, eligibility, exclusions and prizes. Their decisions are final.
5. **Fair play.** No exploiting the arena, sandbox, scoring or other entrants; no collusion between bots; no copying another entrant's bot; no automated abuse of the service (for example mass registration). The organizers may disqualify any bot or person at their discretion.
6. **Eligibility.** Entrants must be of legal age where they live and allowed to receive the prize under the laws that apply to them, including sanctions and export rules. The competition is void where prohibited. Organizers and the people running the arena cannot win. At most one prize per person or team.
7. **Payment.** Method, currency and timing are chosen by the organizers. Winners may be asked for identity and payment details, and are responsible for any taxes and fees on what they receive.
8. **Your bot.** You keep ownership of your bot. You allow the organizers to run, store and analyze it and to publish its name, scores, rankings and hand histories. Bot code is not published without your consent.
9. **The service.** The arena, the tools and SHARINGAN-1 are provided "as is", without warranties. Outages, bugs, resets or data loss may happen; the organizers may re-run, correct or remove results. To the extent the law allows, the organizers are not liable for any loss or damage connected to the competition.
10. **Not real-money poker.** Real-money poker sites ban bots. Nothing here is advice or permission to use a bot anywhere else.
