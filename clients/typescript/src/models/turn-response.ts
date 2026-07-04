/**
 * {@link TurnResponse} — the `application/json` body of a non-streaming turn
 * (`POST /v1/conversations/{id}/turns`, `POST /v1/turns`).
 *
 * `cost` is Loom's authoritative priced {@link TurnCost} for this turn —
 * computed once, inline, at turn time — or `null` when no price is configured
 * for the turn's `(provider, model)`; a pricing miss never fails the turn. The
 * streaming counterpart carries the identical value on the terminal
 * `turn_ended` event's `cost` field (see {@link TurnEventKind}), so
 * {@link collect} on a streamed turn agrees with this envelope for the same
 * input. Parsed with Zod as a response body.
 */

import { z } from "zod";

import type { DeepReadonly } from "../deep-readonly.types.js";
import { messageSchema } from "./message.js";
import { turnCostSchema } from "./turn-cost.js";

/** The `application/json` body of a non-streaming turn. */
export const turnResponseSchema = z.object({
  message: messageSchema,
  cost: turnCostSchema.nullable(),
});

/** The `application/json` body of a non-streaming turn. */
export type TurnResponse = DeepReadonly<z.infer<typeof turnResponseSchema>>;
