/** {@link CollectedTurn} — the payload {@link collect} assembles from a stream. */

import type { Message } from "./models/message.js";
import type { StopReason } from "./models/turn-event.js";
import type { Usage } from "./models/usage.js";

/**
 * The final assistant message, usage snapshot, and stop reason reassembled from
 * a streamed turn — the same information a non-streaming `client.turn()` call
 * returns directly, for a caller who only wants the end result of a stream.
 */
export interface CollectedTurn {
  /** The reassembled assistant message, content parts in ascending index order. */
  readonly message: Message;
  /** The turn's final usage snapshot (an empty object if the stream reported none). */
  readonly usage: Usage;
  /** Why the provider stopped generating. */
  readonly stopReason: StopReason;
}
