/**
 * {@link TurnEvent} — one normalised streaming event from a turn.
 *
 * The gateway maps each provider's native SSE frames onto a provider-agnostic
 * `TurnEventKind` (started, part-started, delta, part-complete, usage, ended,
 * other) while preserving the original frame in `raw`. Each SSE `data:` payload
 * decodes to one of these, so it is validated with Zod as untrusted input.
 */

import { z } from "zod";

import type { DeepReadonly } from "../deep-readonly.types.js";
import { contentDeltaSchema } from "./content-delta.js";
import { contentPartSchema } from "./content-part.js";
import { usageSchema } from "./usage.js";

/**
 * The reason a provider stopped generating a turn. Common values are
 * `end_turn`, `max_tokens`, `stop_sequence`, `tool_use`, `pause_turn` and
 * `refusal`, but providers may report others, so it is left as a free string.
 */
export const stopReasonSchema = z.string();

/** The reason a provider stopped generating a turn. */
export type StopReason = z.infer<typeof stopReasonSchema>;

/** The normalised, provider-agnostic classification of a {@link TurnEvent}. */
export const turnEventKindSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("turn_started") }),
  z.object({
    type: z.literal("content_part_started"),
    index: z.number(),
    part: contentPartSchema,
  }),
  z.object({
    type: z.literal("content_part_delta"),
    index: z.number(),
    delta: contentDeltaSchema,
  }),
  z.object({
    type: z.literal("content_part_complete"),
    index: z.number(),
    part: contentPartSchema,
  }),
  usageSchema.extend({ type: z.literal("usage") }),
  z.object({
    type: z.literal("turn_ended"),
    stop_reason: stopReasonSchema,
    usage: usageSchema.optional(),
  }),
  z.object({
    type: z.literal("other"),
    native_type: z.string().optional(),
  }),
]);

/** The normalised, provider-agnostic classification of a {@link TurnEvent}. */
export type TurnEventKind = DeepReadonly<z.infer<typeof turnEventKindSchema>>;

/**
 * A single streaming event: a normalised {@link TurnEventKind} envelope plus the
 * verbatim native provider event it was derived from.
 */
export const turnEventSchema = z.object({
  kind: turnEventKindSchema,
  raw: z.unknown(),
});

/** A single streaming event. Each SSE `data:` frame decodes to one of these. */
export type TurnEvent = DeepReadonly<z.infer<typeof turnEventSchema>>;
