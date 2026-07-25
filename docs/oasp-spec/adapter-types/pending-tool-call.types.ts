/**
 * A blocking tool use a session is currently parked on, as enumerated
 * by {@link import('./agent-provider.types').AgentProvider.getPendingToolCalls}.
 * A conformant adapter MUST preserve every field here faithfully —
 * `drain` cannot execute a tool call it cannot correctly identify, and
 * (per issue #9) cannot authorize one it cannot correctly attribute to
 * an MCP server either.
 *
 * @see docs/spec/adapters.md § `getPendingToolCalls`
 */
export interface PendingToolCall {
  /** Correlates this tool use to the `sendToolResult` call that resolves it. */
  readonly toolUseId: string;
  /** Name of the tool being invoked (custom or builtin). */
  readonly name: string;
  /** The input arguments passed to the tool. */
  readonly input: Readonly<Record<string, unknown>>;
  /**
   * The `serverUrl` of the MCP server this call was routed through, if
   * the adapter can identify one — provenance `drain`'s pre-dispatch
   * authorization step (`docs/spec/interactions.md` § `drain`) uses to
   * match the call against a granted `mcp` tool's `serverUrl` and
   * `toolAllowlist`. Absent for a `builtin_toolset`/`custom` call, or
   * for an MCP integration whose adapter genuinely cannot attribute a
   * call to a specific server — see `docs/spec/adapters.md`'s "MAY
   * lose" clause 5, which still permits that absence. When present, a
   * conformant adapter MUST preserve it faithfully, exactly like
   * `toolUseId`/`name`/`input`.
   */
  readonly mcpServerUrl?: string;
}
