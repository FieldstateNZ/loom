/**
 * {@link McpServerRef} — a reference to an MCP server the model may use.
 *
 * A request-only shape (the caller supplies it; it is never parsed from a
 * response), so it is a plain readonly type. A bare `name` is the common form:
 * the gateway resolves the URL and token server-side from the tenant's
 * registered servers, so secrets never travel through the client.
 */

/** A reference to an external MCP server the model may use. */
export interface McpServerRef {
  /** The registered server name the gateway resolves, or an inline label. */
  readonly name: string;
  /** An explicit server URL for the inline/advanced form. */
  readonly url?: string;
  /** An explicit authorization token for the inline form. */
  readonly authorization?: string;
  /** Provider-specific tool configuration passed through verbatim. */
  readonly tool_configuration?: unknown;
}
