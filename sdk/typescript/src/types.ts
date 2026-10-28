// Plain types for every payload in docs/TABLE_API.md. Field names are the wire names (snake_case): no conversion layer.

export type Card = string; // "Ah", "Td", "2c"
export type Street = "preflop" | "flop" | "turn" | "river";
export type PlayerStatus = "active" | "folded" | "all_in" | "out";
export type ActionType = "fold" | "check" | "call" | "bet" | "raise";

export interface CallOption { to: number; added: number; all_in: boolean }
export interface RaiseOption { min_to: number; max_to: number }
export interface Legal {
  fold: boolean;
  check: boolean;
  call: CallOption | null;
  raise: RaiseOption | null;
}

export interface ActionEvent {
  hand_id: string; seq: number; action_id: string; street: Street; seat: number;
  type: "post_sb" | "post_bb" | "post_ante" | "post_straddle" | "fold" | "check" | "call" | "bet" | "raise";
  to: number; added: number; all_in: boolean; forced: boolean; timeout: boolean;
  stack: number; pot: number; current_bet: number; min_raise_to: number; next_to_act: number | null;
}

export interface PlayerState {
  seat: number; name: string; stack_start: number; stack: number;
  street_contrib: number; hand_contrib: number; status: PlayerStatus;
}

export interface Pot { amount: number; eligible_seats: number[]; winners?: number[]; share?: number[] }

/** The decision state handed to `Bot.act`: the `your_turn` data + envelope fields (table_id, t_ms; hand_id/seq if missing). */
export interface YourTurn {
  hand_id: string; seq: number; decision_id: string; seat: number; street: Street;
  hole_cards: [Card, Card]; board: Card[];
  history: ActionEvent[]; players: PlayerState[];
  button_seat: number; small_blind: number; big_blind: number;
  pot: number; pots: Pot[]; legal: Legal;
  deadline_ms: number; time_bank_ms: number;
  table_id: string; t_ms: number;
}

export interface HandStartPlayer { seat: number; player_id: string; name: string; stack_start: number; in_hand: boolean }
export interface HandStart {
  hand_id: string; hand_no: number; match_id: string | null; deal_no: number | null; rotation: number | null;
  button_seat: number; sb_seat: number; bb_seat: number; small_blind: number; big_blind: number; ante: number;
  players: HandStartPlayer[]; deck_commitment: string;
}
export interface StreetEvent { hand_id: string; street: Street; board: Card[]; new_cards: Card[]; pot: number; pots: Pot[] }
export interface HandEnd {
  hand_id: string; seq: number; board: Card[];
  showdown: Array<{ seat: number; cards: [Card, Card]; shown: boolean; hand_rank: string; best5: Card[] }>;
  hole_cards: Record<string, [Card, Card]>; // keys are seat numbers as strings (JSON)
  pots: Pot[];
  results: Array<{ seat: number; player_id: string; net: number; stack_end: number }>;
  deck_reveal: { deck: string[]; salt: string };
  ended_by: "showdown" | "fold";
}

export interface Seat { seat: number; player_id: string | null; name: string | null; stack: number; status: "empty" | "seated" | "sitting_out" | "disconnected" }
export interface TableConfig {
  variant: "nlhe"; max_seats: number; small_blind: number; big_blind: number; ante: number; straddle: boolean;
  starting_stack: number; stack_mode: "reset" | "carry"; decision_time_ms: number; time_bank_ms: number;
  timeout_action: "check_fold"; rake: { percent: number; cap: number }; min_players_to_deal: number;
  odd_chip_rule: string; show_mode: string; mode?: "sandbox";
}
export interface Table {
  table_id: string; config: TableConfig; status: "waiting" | "running" | "paused" | "closed";
  seats: Seat[]; button_seat: number | null; hand_id: string | null; table_seq: number;
  hand?: unknown | null;
}
export interface Player { player_id: string; name: string; kind: "bot" | "human"; owner?: string; created_at: string; token?: string }
export interface ApiError { code: string; message: string; decision_id?: string; details?: Record<string, unknown> }

/** What a bot returns. `to` is the TOTAL street contribution after the action ("to", never "by"). */
export interface Action { type: ActionType | "all_in"; to?: number }

export interface Envelope<D = any> {
  v: 1; type: string; table_id?: string; hand_id?: string; seq?: number; table_seq?: number; t_ms: number; data: D;
}
