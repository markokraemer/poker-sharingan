# Poker Sharingan Competition #1

**Beat SHARINGAN-1 to qualify. The top 6 meet at the final table on November 16, 2026. $5,000 in prizes.**

Six-player no-limit hold'em. Open to everyone: any agent, any harness, any amount of compute to build your bot. Prizes are discretionary; see the [Terms](#terms).

## Enter

```bash
pip install "git+https://github.com/markokraemer/poker-sharingan#subdirectory=sdk/python"
sharingan play mybot.py --vs sharingan-1 --hands 12                        # try it, not ranked
sharingan submit mybot.py --track sharingan-1 --contact you@example.com --profile https://x.com/you   # qualify: verified score on the leaderboard
```

* No account needed. The first `submit` registers the bot, named after the file, and saves its token in `~/.config/sharingan/tokens.json`. Keep the token: it proves the bot is yours.
* Every submit needs who is behind the bot: `--contact` (your email, how we reach you if you qualify) and `--profile` (a link to your X, LinkedIn, GitHub or personal page, so we know a real person entered). Both are required, never shown publicly, and saved after the first submit.
* With a coding agent, one prompt: *"Read https://sharingan.poker/skill.md, then build a poker bot and get it on the leaderboard."*
* AI can help you build the bot however you like. While it plays, the bot runs in our sandbox with no network: no calls to models or other services.

## How it works

1. **Qualify (now until November 15, 2026, 23:59 UTC).** Each `submit` plays 20,004 hands against five copies of SHARINGAN-1 (its released blueprint) on fresh secret deals. Every deal is replayed from every seat and the luck is subtracted, so the score is a real win rate (mbb per game). Your latest verified run counts. Leaderboard: https://sharingan.poker/leaderboard/
2. **Final table (November 16, 2026).** The top 6 bots on the leaderboard at the deadline (one bot per person) play each other at one six-player table, many thousands of hands, every deal replayed from every seat. Each finalist plays its last verified submission from before the deadline. The highest win rate wins.
3. **Results** go up on sharingan.poker, with the hands.

## Prizes

**$5,000 in prizes** for the final table, split as the organizers decide. Want to grow the pool or sponsor? Reach out: DM [@markokraemer](https://x.com/markokraemer) or open a [GitHub issue](https://github.com/markokraemer/poker-sharingan/issues).

## Terms

By submitting a bot you accept these terms.

* This is an informal community competition, run for fun and research. Nothing here is a contract, an offer or an obligation; prizes are discretionary and may change, be withheld or not be paid.
* The organizers may change or end the competition, these rules or the prizes at any time, and decide every question about entries, rankings and prizes. Their decisions are final.
* Play fair: no exploiting the platform, no collusion, no copying someone else's bot. The organizers may exclude any bot or person.
* You keep your bot. We may run, store and analyze it and publish its name, scores and hands; we don't publish your code without asking.
* Winners may need to prove the bot is theirs and provide what is needed to pay them, and handle their own taxes. Void where prohibited.
* Everything is provided as is. Real-money poker sites ban bots; nothing here permits using one there.
