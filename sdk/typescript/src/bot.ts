import type { Action, ActionEvent, ApiError, HandEnd, HandStart, StreetEvent, YourTurn } from "./types.js";

/** Implement `act`; override the optional hooks to track hands. */
export interface BotLike {
  name?: string;
  /** true = send exactly what act() returned (the server may reject it). Default: the runner legalizes. */
  strict?: boolean;
  act(state: YourTurn): Action | Promise<Action>;
  onConnect?(welcome: Record<string, unknown>): void | Promise<void>;
  onHandStart?(hand: HandStart): void | Promise<void>;
  onAction?(event: ActionEvent): void | Promise<void>;
  onStreet?(street: StreetEvent): void | Promise<void>;
  onHandEnd?(end: HandEnd): void | Promise<void>;
  onError?(error: ApiError): void | Promise<void>;
}

/** Optional base class (a plain object with `act` works too). */
export abstract class Bot implements BotLike {
  name = "bot";
  strict = false;
  abstract act(state: YourTurn): Action | Promise<Action>;
}
