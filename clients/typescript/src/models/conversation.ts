/**
 * {@link Conversation} — a tenant-scoped conversation as returned by
 * create/fetch.
 *
 * Carries the provider/model binding, the persisted message history, and
 * bookkeeping timestamps. Parsed with Zod because it is a response body. Some
 * fields are nullable (the gateway distinguishes "absent" from "explicitly
 * null"), so those accept `null` as well as being optional.
 */

import { z } from "zod";

import type { DeepReadonly } from "../deep-readonly.types.js";
import { cacheHintSchema } from "./cache.js";
import { messageSchema } from "./message.js";

/** A tenant-scoped conversation, as returned by create/fetch. */
export const conversationSchema = z.object({
  id: z.string(),
  tenant_id: z.string(),
  binding: z.object({ provider: z.string(), model: z.string() }),
  system: z.string().nullish(),
  system_cache: cacheHintSchema.nullish(),
  messages: z.array(messageSchema),
  metadata: z.unknown().optional(),
  created_at: z.string().optional(),
  updated_at: z.string().optional(),
});

/** A tenant-scoped conversation, as returned by create/fetch. */
export type Conversation = DeepReadonly<z.infer<typeof conversationSchema>>;
