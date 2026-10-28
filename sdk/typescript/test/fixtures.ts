import type { YourTurn } from "../src/index.js";

// Worked example from docs/TABLE_API.md section 5.3 (seat 0 on the flop, facing a check)
export const DOC_YOUR_TURN = {
  v: 1, type: "your_turn", table_id: "t1", hand_id: "h-77", seq: 9, t_ms: 1792000000000,
  data: {
    decision_id: "d-77-9", seat: 0, street: "flop", hole_cards: ["Ah", "Kd"], board: ["7c", "8d", "2s"],
    history: [
      { seq: 1, street: "preflop", seat: 4, type: "post_sb", to: 50, added: 50, all_in: false, forced: true, timeout: false, stack: 9950, pot: 50, current_bet: 50, min_raise_to: 100, next_to_act: 5 },
      { seq: 8, street: "flop", seat: 5, type: "check", to: 0, added: 0, all_in: false, forced: false, timeout: false, stack: 9750, pot: 550, current_bet: 0, min_raise_to: 100, next_to_act: 0 },
    ],
    players: [
      { seat: 0, name: "pluribus", stack_start: 10000, stack: 9750, street_contrib: 0, hand_contrib: 250, status: "active" },
      { seat: 5, name: "erin", stack_start: 10000, stack: 9750, street_contrib: 0, hand_contrib: 250, status: "active" },
    ],
    button_seat: 3, small_blind: 50, big_blind: 100, pot: 550, pots: [{ amount: 550, eligible_seats: [0, 5] }],
    legal: { fold: true, check: true, call: null, raise: { min_to: 100, max_to: 9750 } },
    deadline_ms: 1792000005000, time_bank_ms: 30000,
  },
};

export const docState = (): YourTurn => {
  const e = structuredClone(DOC_YOUR_TURN);
  return { ...e.data, hand_id: e.hand_id, seq: e.seq, table_id: e.table_id, t_ms: e.t_ms } as unknown as YourTurn;
};

export const facingBetState = (): YourTurn => {
  const s = docState();
  s.pot = 750;
  s.players[1]!.street_contrib = 200;
  s.legal = { fold: true, check: false, call: { to: 200, added: 200, all_in: false }, raise: { min_to: 400, max_to: 9750 } };
  return s;
};
