import type { Event } from '@oasp/schemas';

/**
 * Pagination input to
 * {@link import('./agent-provider.types').AgentProvider.listSessionEvents}.
 * `afterId` is opaque to the caller — it is always a previously-seen
 * `Event.id`, never constructed by hand — matching the S1 rule that
 * `listSessionEvents` pagination relies on `id`'s lexicographic
 * monotonicity as its cursor.
 */
export interface ListSessionEventsOptions {
  /** Return only events emitted strictly after this event id. Omit to start from the beginning of the session's history. */
  readonly afterId?: string;
  /** Maximum number of events to return in this page. */
  readonly limit?: number;
}

/**
 * A page of a session's normalised event history, in emission order.
 */
export interface ListSessionEventsResult {
  /** The events in this page, ordered by emission (equivalently, by lexicographic `id`). */
  readonly events: readonly Event[];
  /** The `id` to pass as the next call's `afterId` to continue pagination, or `null` if this page reached the end of the currently-known history. */
  readonly nextCursor: string | null;
}
