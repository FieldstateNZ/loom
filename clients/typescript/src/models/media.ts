/**
 * Media sources and citations carried by image/document content parts.
 *
 * The gateway echoes these back verbatim inside conversation history, so they
 * are validated with Zod at the boundary.
 */

import { z } from "zod";

import type { DeepReadonly } from "../deep-readonly.types.js";

/** The origin of an image or document's bytes: inline base64 or a fetchable URL. */
export const mediaSourceSchema = z.discriminatedUnion("type", [
  z.object({
    type: z.literal("base64"),
    media_type: z.string(),
    data: z.string(),
  }),
  z.object({
    type: z.literal("url"),
    url: z.string(),
  }),
]);

/** The origin of an image or document's bytes: inline base64 or a fetchable URL. */
export type MediaSource = DeepReadonly<z.infer<typeof mediaSourceSchema>>;

/**
 * A provider-native citation payload. Its shape varies per provider, so it is
 * preserved verbatim as `unknown` rather than forced into a common schema.
 */
export const citationSchema = z.unknown();

/** A provider-native citation payload, preserved verbatim. */
export type Citation = z.infer<typeof citationSchema>;
