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
import { turnCostSchema } from "./turn-cost.js";
import { usageSchema } from "./usage.js";

/**
 * The reason a provider stopped generating a turn.
 *
 * `StopReason` is a plain (externally tagged) serde enum, not internally
 * tagged like {@link ContentPart}: the known reasons (`end_turn`,
 * `max_tokens`, `stop_sequence`, `tool_use`, `pause_turn`, `refusal`)
 * serialize as bare strings, but a provider-specific reason Loom does not
 * model is wrapped verbatim as `{ other: "..." }` (Rust's
 * `StopReason::Other(String)` tuple variant) — never a bare string. Both
 * shapes are accepted here so an unrecognised reason still parses instead of
 * failing validation.
 */
/**
 * The known stop reasons, kept as editor hints only. The wire may carry others:
 * `StopReason` is `#[non_exhaustive]` in Rust and its unit variants serialize as
 * bare strings, so an unrecognised reason must still parse (see the doc above).
 */
type KnownStopReason =
  | "end_turn"
  | "max_tokens"
  | "stop_sequence"
  | "tool_use"
  | "pause_turn"
  | "refusal";

export const stopReasonSchema = z.union([z.string(), z.object({ other: z.string() })]);

/** The reason a provider stopped generating a turn. */
export type StopReason = KnownStopReason | (string & {}) | { readonly other: string };

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
    // Loom's authoritative priced cost for the turn, injected by the gateway
    // (never set by a provider). Absent — not `null` — when no price is
    // configured for the (provider, model), matching the non-streaming
    // `TurnResponse.cost` for the same input.
    cost: turnCostSchema.optional(),
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
