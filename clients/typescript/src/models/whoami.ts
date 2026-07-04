/**
 * {@link WhoAmI} — the authenticated identity echoed by `GET /v1/whoami`.
 *
 * Lets a caller confirm which tenant and key a virtual key resolves to. Parsed
 * with Zod as a response body.
 */

import { z } from "zod";

import type { DeepReadonly } from "../deep-readonly.types.js";

/** The authenticated identity echoed by `GET /v1/whoami`. */
export const whoAmISchema = z.object({
  tenant_id: z.string(),
  key_id: z.string(),
  key_prefix: z.string(),
  scopes: z.array(z.string()),
});

/** The authenticated identity echoed by `GET /v1/whoami`. */
export type WhoAmI = DeepReadonly<z.infer<typeof whoAmISchema>>;
