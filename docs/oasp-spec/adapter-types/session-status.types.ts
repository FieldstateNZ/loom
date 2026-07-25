import type { Event } from '@oasp/schemas';

/**
 * A session's coarse execution state, as reported by
 * {@link import('./agent-provider.types').AgentProvider.getSessionStatus}.
 *
 * Deliberately *derived* from {@link Event}'s `status` variant rather
 * than hand-redefined — `'running' | 'idle' | 'error'` must never drift
 * from the `status` Event's own vocabulary, since the two describe the
 * same underlying concept from two different angles (a point-in-time
 * query vs. a transition notification).
 */
export type SessionStatus = Extract<Event, { type: 'status' }>['status'];
