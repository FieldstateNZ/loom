/**
 * {@link ContentDelta} — an incremental change to a streaming content part.
 *
 * Emitted only while streaming a turn, one per `content_part_delta` event, so
 * the consumer can render tokens as they arrive. Internally tagged on `type`.
 */

import { z } from "zod";

import type { DeepReadonly } from "../deep-readonly.types.js";
import { citationSchema } from "./media.js";

/** An incremental change to a streaming content part (tagged on `type`). */
export const contentDeltaSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("text"), text: z.string() }),
  z.object({ type: z.literal("json"), partial_json: z.string() }),
  z.object({ type: z.literal("thinking"), thinking: z.string() }),
  z.object({ type: z.literal("signature_delta"), signature: z.string() }),
  z.object({ type: z.literal("citation"), citation: citationSchema }),
]);

/** An incremental change to a streaming content part (tagged on `type`). */
export type ContentDelta = DeepReadonly<z.infer<typeof contentDeltaSchema>>;
