/** {@link CollectedTurn} — the payload {@link collect} assembles from a stream. */

import type { Message } from "./models/message.js";
import type { StopReason } from "./models/turn-event.js";
import type { TurnCost } from "./models/turn-cost.js";
import type { Usage } from "./models/usage.js";

/**
 * The final assistant message, usage snapshot, stop reason, and priced cost
 * reassembled from a streamed turn — the same information a non-streaming
 * `client.turn()` call returns directly (its `TurnResponse`), for a caller who
 * only wants the end result of a stream.
 */
export interface CollectedTurn {
  /** The reassembled assistant message, content parts in ascending index order. */
  readonly message: Message;
  /** The turn's final usage snapshot (an empty object if the stream reported none). */
  readonly usage: Usage;
  /** Why the provider stopped generating. */
  readonly stopReason: StopReason;
  /**
   * Loom's authoritative priced cost for the turn, from the terminal
   * `turn_ended` event's `cost` — `null` when no price is configured for the
   * turn's `(provider, model)`, the same as the non-streaming
   * `TurnResponse.cost` for the same input.
   */
  readonly cost: TurnCost | null;
}
