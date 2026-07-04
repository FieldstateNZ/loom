/**
 * Prompt-cache primitives shared across content and tool definitions.
 *
 * A {@link CacheHint} marks a breakpoint in a prompt that a provider may cache
 * and reuse on later turns, trading a small write cost now for cheaper reads
 * later. These appear on content parts the gateway echoes back, so they are
 * parsed with Zod rather than trusted.
 */

import { z } from "zod";

import type { DeepReadonly } from "../deep-readonly.types.js";

/** How long a provider should keep a cache entry alive after it is written. */
export const cacheTtlSchema = z.enum(["five_minutes", "one_hour"]);

/** How long a provider should keep a cache entry alive after it is written. */
export type CacheTtl = z.infer<typeof cacheTtlSchema>;

/** A provider-agnostic prompt-cache breakpoint. */
export const cacheHintSchema = z.object({
  ttl: cacheTtlSchema.optional(),
});

/** A provider-agnostic prompt-cache breakpoint. */
export type CacheHint = DeepReadonly<z.infer<typeof cacheHintSchema>>;
