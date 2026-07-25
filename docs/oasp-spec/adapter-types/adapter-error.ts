import type { DomainError } from '../shared/domain-error.types';

/**
 * The error shape every {@link import('./agent-provider.types').AgentProvider}
 * operation returns on the failure branch of its {@link import('../shared/result').Result}.
 * `retryable` lets a server distinguish "try again" failures (a
 * transient provider hiccup) from "this will never succeed as posed"
 * failures (an unknown session id, a malformed input) without parsing
 * `code` — MUST be set deliberately by every adapter, never defaulted.
 */
export interface AdapterError extends DomainError {
  readonly retryable: boolean;
}

/**
 * Factory functions for the `AdapterError`s the mock provider (and any
 * other `AgentProvider` implementation) needs. Centralising these
 * keeps `code` values stable across call sites instead of each one
 * hand-rolling its own error literal.
 */
export const adapterErrors = {
  /** No session exists with the given id. */
  sessionNotFound: (sessionId: string): AdapterError => ({
    code: 'Adapter.SessionNotFound',
    message: `No session found with id "${sessionId}".`,
    retryable: false,
  }),
  /** No provider-side agent exists with the given id. */
  agentNotFound: (providerAgentId: string): AdapterError => ({
    code: 'Adapter.AgentNotFound',
    message: `No provider agent found with id "${providerAgentId}".`,
    retryable: false,
  }),
  /** `sendToolResult` was called with a `toolUseId` that has no matching pending tool use. */
  unknownToolUse: (toolUseId: string): AdapterError => ({
    code: 'Adapter.UnknownToolUse',
    message: `No pending tool use found with id "${toolUseId}".`,
    retryable: false,
  }),
  /** `listSessionEvents` (or the underlying transcript fetch it powers) failed. Used to exercise migrate's degrade-to-fresh-start path. */
  transcriptFetchFailed: (sessionId: string): AdapterError => ({
    code: 'Adapter.TranscriptFetchFailed',
    message: `Failed to fetch the transcript for session "${sessionId}".`,
    retryable: true,
  }),
  /** A generic, test-induced failure — used by the mock provider's controls to simulate an arbitrary operation failing on demand. */
  induced: (operation: string): AdapterError => ({
    code: 'Adapter.InducedFailure',
    message: `Induced failure for operation "${operation}" (test-controlled).`,
    retryable: true,
  }),
} as const;
