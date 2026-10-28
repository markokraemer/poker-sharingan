from sharingan import Action, ArenaClient, Bot, run

class MyBot(Bot):
    def act(self, s):                      # s = the your_turn payload
        if s.legal.raise_ and s.hole_cards[0][0] in "AK":
            return Action.raise_to(3 * s.big_blind)   # "to" amount, clamped to legal for you
        return Action.call() if s.legal.call and s.pot_odds < 0.25 else Action.check() if s.legal.check else Action.fold()

api = ArenaClient("https://arena.sharingan.poker")
me = api.register_bot("my-bot")                         # token shown once
api.join_table("TABLE_ID", seat=0, player_id=me.player_id)
run(MyBot(), "https://arena.sharingan.poker", me.token)  # connects, reconnects, answers every your_turn
