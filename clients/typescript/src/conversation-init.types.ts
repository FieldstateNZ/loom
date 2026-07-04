/** {@link ConversationInit} — the options for opening a conversation. */

import type { ConversationOptions } from "./models/conversation-options.js";

/** Options for opening a lazily-created, tenant-scoped conversation. */
export interface ConversationInit {
  /** The model id, as the provider expects it (e.g. `claude-haiku-4-5-20251001`). */
  readonly model: string;
  /** The provider to bind to. Defaults to `anthropic`. */
  readonly provider?: string;
  /** An optional system prompt applied to the whole conversation. */
  readonly system?: string;
  /** Free-form caller metadata (tags, correlation ids, …). */
  readonly metadata?: unknown;
  /** Base request options applied to every turn (further chainable). */
  readonly options?: ConversationOptions;
}
