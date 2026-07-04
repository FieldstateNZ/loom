/**
 * {@link UsageRollupResponse} — the aggregated spend/usage report from
 * `GET /v1/usage`.
 *
 * Money fields are strings (decimal, not float) to avoid rounding drift, and
 * `group` is nullable because the "ungrouped total" row has no group key.
 * Parsed with Zod as a response body.
 */

import { z } from "zod";

import type { DeepReadonly } from "../deep-readonly.types.js";

/** The dimension a usage rollup is grouped by. */
export const usageGroupBySchema = z.enum(["key", "model", "conversation"]);

/** The dimension a usage rollup is grouped by. */
export type UsageGroupBy = z.infer<typeof usageGroupBySchema>;

/** One grouped row in a usage-rollup response. */
export const usageRollupRowSchema = z.object({
  group: z.string().nullable(),
  event_count: z.number(),
  input_tokens: z.number(),
  output_tokens: z.number(),
  cache_read_tokens: z.number(),
  cache_write_tokens: z.number(),
  cost: z.string(),
  batch_cost: z.string(),
  interactive_cost: z.string(),
});

/** One grouped row in a usage-rollup response. */
export type UsageRollupRow = DeepReadonly<z.infer<typeof usageRollupRowSchema>>;

/** A usage-rollup response envelope. */
export const usageRollupResponseSchema = z.object({
  group_by: usageGroupBySchema,
  from: z.string().nullable(),
  to: z.string().nullable(),
  rows: z.array(usageRollupRowSchema),
});

/** A usage-rollup response envelope. */
export type UsageRollupResponse = DeepReadonly<z.infer<typeof usageRollupResponseSchema>>;
