/**
 * The resource-usage snapshot a provider reports for a response.
 *
 * Returned inside messages and streaming turn-ended events, so it is parsed
 * with Zod. All counters are optional because not every provider reports every
 * dimension (e.g. cache tokens only appear when caching was in play).
 */

import { z } from "zod";

import type { DeepReadonly } from "../deep-readonly.types.js";

/** A snapshot of the resource usage a provider reported for a response. */
export const usageSchema = z.object({
  input_tokens: z.number().optional(),
  output_tokens: z.number().optional(),
  cache_read_tokens: z.number().optional(),
  cache_write_tokens: z.number().optional(),
  server_tool_use: z.record(z.string(), z.number()).optional(),
  raw: z.unknown().optional(),
});

/** A snapshot of the resource usage a provider reported for a response. */
export type Usage = DeepReadonly<z.infer<typeof usageSchema>>;
