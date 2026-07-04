/**
 * {@link McpServerListResponse} — the response envelope from
 * `GET /v1/mcp-servers`.
 *
 * Carries only the tenant's registered MCP server *names* — never a URL or
 * authorization token, both of which stay server-side. Parsed with Zod as a
 * response body; {@link LoomClient.mcpServers} unwraps it to a bare name list.
 */

import { z } from "zod";

import type { DeepReadonly } from "../deep-readonly.types.js";

/** The response envelope from `GET /v1/mcp-servers`. */
export const mcpServerListResponseSchema = z.object({
  servers: z.array(z.string()),
});

/** The response envelope from `GET /v1/mcp-servers`. */
export type McpServerListResponse = DeepReadonly<z.infer<typeof mcpServerListResponseSchema>>;
