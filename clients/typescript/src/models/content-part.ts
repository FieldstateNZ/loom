/**
 * {@link ContentPart} — one typed piece of a message, internally tagged on
 * `type`.
 *
 * This mirrors Loom's core `ContentPart` serde enum (`crates/loom-core`). It is
 * the richest response shape the client parses — text, media, tool calls and
 * results, thinking blocks — so it is expressed as a Zod discriminated union and
 * validated at the boundary. Provider-specific inputs/results stay `unknown`
 * because their shape is defined by the provider, not the gateway.
 */

import { z } from "zod";

import type { DeepReadonly } from "../deep-readonly.types.js";
import { cacheHintSchema } from "./cache.js";
import { citationSchema, mediaSourceSchema } from "./media.js";

/** A single, typed piece of message content (internally tagged on `type`). */
export const contentPartSchema = z.discriminatedUnion("type", [
  z.object({
    type: z.literal("text"),
    text: z.string(),
    citations: z.array(citationSchema).optional(),
    cache: cacheHintSchema.optional(),
  }),
  z.object({
    type: z.literal("image"),
    source: mediaSourceSchema,
    cache: cacheHintSchema.optional(),
  }),
  z.object({
    type: z.literal("document"),
    source: mediaSourceSchema,
    cache: cacheHintSchema.optional(),
  }),
  z.object({
    type: z.literal("tool_use"),
    id: z.string(),
    name: z.string(),
    input: z.unknown(),
    cache: cacheHintSchema.optional(),
  }),
  z.object({
    type: z.literal("tool_result"),
    tool_use_id: z.string(),
    content: z.unknown(),
    is_error: z.boolean().optional(),
    cache: cacheHintSchema.optional(),
  }),
  z.object({
    type: z.literal("server_tool_use"),
    id: z.string(),
    name: z.string(),
    input: z.unknown(),
  }),
  z.object({
    type: z.literal("server_tool_result"),
    tool_use_id: z.string(),
    content: z.unknown(),
  }),
  z.object({
    type: z.literal("thinking"),
    thinking: z.string(),
    signature: z.string().optional(),
    cache: cacheHintSchema.optional(),
  }),
  z.object({
    type: z.literal("redacted_thinking"),
    data: z.string(),
  }),
  z.object({
    type: z.literal("provider_extension"),
    provider: z.string(),
    kind: z.string(),
    payload: z.unknown(),
  }),
]);

/** A single, typed piece of message content (internally tagged on `type`). */
export type ContentPart = DeepReadonly<z.infer<typeof contentPartSchema>>;
