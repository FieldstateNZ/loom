import type { AgentDefinition, Deployment, Event, Session } from '@oasp/schemas';
import type { Result } from '../shared/result';
import type { AdapterError } from './adapter-error';
import type { CreateSessionOptions } from './create-session-options.types';
import type { EnsureEnvironmentResult } from './ensure-environment.types';
import type { ListSessionEventsOptions, ListSessionEventsResult } from './list-session-events.types';
import type { PendingToolCall } from './pending-tool-call.types';
import type { SessionStatus } from './session-status.types';

/**
 * OASP v0's adapter contract: the interface every provider integration
 * (Anthropic, and — once implemented — OpenAI/Google) must satisfy so
 * a server can drive it without knowing which provider is underneath.
 * This is the executable form of `docs/spec/adapters.md`, which states
 * the full normative behaviour — including the preserve-vs-may-lose
 * boundary — for every method below; the TSDoc here is a summary, not
 * a substitute.
 *
 * Every operation returns a {@link Result} rather than throwing for
 * expected failures (unknown session, rejected precondition) —
 * `streamEvents` is the sole exception, since an `AsyncIterable`
 * has no natural place to carry a `Result` wrapper; a `streamEvents`
 * implementation surfaces failures as `error` {@link Event}s within the
 * stream itself instead.
 *
 * @see docs/spec/adapters.md
 * @see docs/oasp-v0-concept.md § Adapter contract
 */
export interface AgentProvider {
  /**
   * Idempotently ensures the named provider-side environment exists.
   * MUST be safe to call repeatedly for the same `environmentId`.
   */
  ensureEnvironment(environmentId: string): Promise<Result<EnsureEnvironmentResult, AdapterError>>;

  /**
   * Materializes the given `AgentDefinition` at the provider within
   * `environmentId`, returning the resulting `Deployment`.
   */
  createAgent(definition: AgentDefinition, environmentId: string): Promise<Result<Deployment, AdapterError>>;

  /**
   * Updates the provider-side agent identified by `providerAgentId` to
   * match `definition`, in place — MUST NOT create a second agent.
   */
  updateAgent(
    providerAgentId: string,
    definition: AgentDefinition,
    environmentId: string,
  ): Promise<Result<Deployment, AdapterError>>;

  /** Fetches the current `Deployment` for `providerAgentId` without mutating provider state. */
  getAgent(providerAgentId: string): Promise<Result<Deployment, AdapterError>>;

  /**
   * Creates a new provider execution context pinned to
   * `options.pinnedAgentVersion`, with `options.resources` and
   * `options.vaultIds` mounted in full, and — if `options.seed` is
   * present — the given transcript seeded as already-exchanged
   * content. MUST preserve version pinning, resource/vault fidelity,
   * and the "no unsolicited assistant turn from seeding" guarantee;
   * see `docs/spec/adapters.md` § `createSession`.
   */
  createSession(options: CreateSessionOptions): Promise<Result<Session, AdapterError>>;

  /** Posts `content` into the named session as a new turn, attributed to `principal` where the provider supports it. */
  sendMessage(sessionId: string, content: string, principal?: string): Promise<Result<void, AdapterError>>;

  /**
   * Posts `result` for the pending tool use identified by
   * `toolUseId`. MUST reject (never silently no-op) if `toolUseId`
   * does not correspond to a currently pending tool use.
   */
  sendToolResult(sessionId: string, toolUseId: string, result: unknown): Promise<Result<void, AdapterError>>;

  /** Reports the session's current coarse status: `running`, `idle`, or `error`. */
  getSessionStatus(sessionId: string): Promise<Result<SessionStatus, AdapterError>>;

  /**
   * Returns a page of the session's normalised event history, in
   * emission order, paginated via `options`. MUST agree exactly with
   * `streamEvents`'s emission order for the same session.
   */
  listSessionEvents(
    sessionId: string,
    options?: ListSessionEventsOptions,
  ): Promise<Result<ListSessionEventsResult, AdapterError>>;

  /**
   * Streams the session's events in true emission order. MUST
   * terminate once a `status: 'idle'` or `recoverable: false` `error`
   * Event has been yielded, and MUST NOT terminate merely because
   * output paused while `status` remains `'running'`.
   */
  streamEvents(sessionId: string): AsyncIterable<Event>;

  /**
   * Enumerates every blocking tool use the session is currently parked
   * on. MUST be complete — `drain`'s correctness depends on it. MUST
   * return an empty array (never an error) when nothing is pending.
   */
  getPendingToolCalls(sessionId: string): Promise<Result<readonly PendingToolCall[], AdapterError>>;
}
