/** {@link StatelessTurnInit} — the input for a non-persisted turn. */

import type { ConversationOptions } from "./models/conversation-options.js";
import type { Message } from "./models/message.js";

/**
 * A stateless turn request (`POST /v1/turns`): the caller supplies the whole
 * message history each time and nothing is persisted server-side.
 */
export interface StatelessTurnInit {
  /** The model id, as the provider expects it. */
  readonly model: string;
  /** The provider to bind to. Defaults to `anthropic`. */
  readonly provider?: string;
  /** An optional system prompt for this turn. */
  readonly system?: string;
  /** The full conversation history to send. */
  readonly messages: readonly Message[];
  /** Request options shaping generation. */
  readonly options?: ConversationOptions;
}
