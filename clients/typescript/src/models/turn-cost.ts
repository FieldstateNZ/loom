/**
 * {@link TurnCost} — Loom's authoritative priced cost for a turn.
 *
 * Computed inline, server-side, at turn time from the gateway's pricing table
 * (`(provider, model)` rate lookup) — the same figure recorded to the usage
 * outbox, not `GET /v1/usage`'s eventually-consistent aggregate. `amount` is a
 * decimal string (not a float) to avoid rounding drift, matching the
 * usage-rollup cost fields. Parsed with Zod as part of a response body.
 */

import { z } from "zod";

import type { DeepReadonly } from "../deep-readonly.types.js";

/** Loom's authoritative priced cost for a turn. */
export const turnCostSchema = z.object({
  amount: z.string(),
  currency: z.string(),
});

/** Loom's authoritative priced cost for a turn. */
export type TurnCost = DeepReadonly<z.infer<typeof turnCostSchema>>;
