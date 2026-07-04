/**
 * {@link ConversationOptions} — the per-turn generation options a caller
 * accumulates (temperature, tool offers, caching, MCP refs, …).
 *
 * Request-only: sent on every turn, never parsed from a response, so it is a
 * plain readonly type. The fluent builder mutates a private draft of this shape
 * and hands back a frozen snapshot via `buildOptions()`.
 */

import type { McpServerRef } from "./mcp-server-ref.js";
import type { ServerTool } from "./server-tool.js";
import type { ToolDefinition } from "./tool-definition.js";

/**
 * How a provider should treat cache hints on a model that cannot cache:
 * `soft_ignore` drops them silently; `hard_fail` rejects the request.
 */
export type CacheNegotiation = "soft_ignore" | "hard_fail";

/** Options that shape how a provider should generate a response. */
export interface ConversationOptions {
  /** Sampling temperature (higher is more random). */
  readonly temperature?: number;
  /** The maximum number of tokens the model may generate. */
  readonly max_tokens?: number;
  /** Sequences that, if generated, stop the turn. */
  readonly stop_sequences?: readonly string[];
  /** Client-side tools the model may call. */
  readonly tools?: readonly ToolDefinition[];
  /** Provider-executed tools the model may use. */
  readonly server_tools?: readonly ServerTool[];
  /** Enables Loom's automatic prompt caching. */
  readonly auto_cache?: boolean;
  /** How to treat cache hints on a non-caching model. */
  readonly cache_negotiation?: CacheNegotiation;
  /** External MCP servers the model may reach. */
  readonly mcp_servers?: readonly McpServerRef[];
  /** Provider-specific options passed through verbatim. */
  readonly provider_options?: Record<string, unknown>;
}
