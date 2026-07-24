/**
 * Result of {@link import('./agent-provider.types').AgentProvider.ensureEnvironment}:
 * confirmation that the named environment exists and is ready to host
 * provider-side agents.
 */
export interface EnsureEnvironmentResult {
  /** Echoes the `environmentId` the caller requested — confirms which environment was ensured. */
  readonly environmentId: string;
}
