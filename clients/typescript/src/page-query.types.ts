/** {@link PageParams} — offset/limit paging for conversation-history reads. */

/**
 * A page window over a conversation's message history. Both fields are optional;
 * omitting them asks the gateway for its default page.
 */
export interface PageParams {
  /** The maximum number of messages to return. */
  readonly limit?: number;
  /** The number of messages to skip from the start of the history. */
  readonly offset?: number;
}
