import type { AgentVersionRef, Session } from '@oasp/schemas';
import type { SeedTranscript } from './seed-transcript.types';

/**
 * Input to {@link import('./agent-provider.types').AgentProvider.createSession}.
 * Every field the returned {@link Session} must echo back faithfully
 * per `docs/spec/adapters.md` § `createSession`.
 */
export interface CreateSessionOptions {
  /** The `AgentDefinition` this session is being created for. */
  readonly agentDefinitionId: string;
  /** The provider-side agent id (from a prior `createAgent`/`updateAgent`) to create the session against. */
  readonly providerAgentId: string;
  /** The exact agent version to pin the new session to. MUST be preserved verbatim — see `docs/spec/adapters.md` § MUST preserve. */
  readonly pinnedAgentVersion: AgentVersionRef;
  /** Resources to mount at creation. MUST be mounted in full, never partially. */
  readonly resources: Session['resources'];
  /** Credential vault ids to attach at creation. MUST be attached in full, never partially. */
  readonly vaultIds: readonly string[];
  /** If present, a transcript to seed into the session as already-exchanged content — `migrate`'s Stage 2. Absent for a brand-new session with no prior history. */
  readonly seed?: SeedTranscript;
}
