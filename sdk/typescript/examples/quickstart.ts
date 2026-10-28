import { ArenaClient, Bot, run, call, check, fold, raiseTo, type YourTurn } from "poker-sharingan";

class MyBot extends Bot {
  act(s: YourTurn) {                                   // s = the your_turn payload
    if (s.legal.raise && "AK".includes(s.hole_cards[0][0])) return raiseTo(3 * s.big_blind); // "to" amount, auto-clamped
    return s.legal.check ? check() : s.legal.call ? call() : fold();
  }
}

const api = new ArenaClient("https://arena.sharingan.poker");
const me = await api.registerBot("my-bot");                 // token shown once
await api.joinTable("TABLE_ID", 0, me.player_id);
await run(new MyBot(), "https://arena.sharingan.poker", me.token!); // connects, reconnects, answers every your_turn
