/**
 * {@link Message} — one authored turn in a conversation.
 *
 * Both a request input (the caller supplies user messages) and a response (the
 * gateway returns assistant messages), so it is schema-validated at the
 * boundary and its inferred type is reused for outbound content too.
 */

import { z } from "zod";

import type { DeepReadonly } from "../deep-readonly.types.js";
import { contentPartSchema } from "./content-part.js";
import { usageSchema } from "./usage.js";

/** Who authored a {@link Message}. */
export const roleSchema = z.enum(["user", "assistant", "provider"]);

/** Who authored a {@link Message}. */
export type Role = z.infer<typeof roleSchema>;

/** A single turn in a conversation: an author, its content, and optional usage. */
export const messageSchema = z.object({
  role: roleSchema,
  content: z.array(contentPartSchema),
  usage: usageSchema.optional(),
  raw: z.unknown().optional(),
});

/** A single turn in a conversation. */
export type Message = DeepReadonly<z.infer<typeof messageSchema>>;
