/**
 * `@fieldstate/loom-client` — a TypeScript client for Loom, Fieldstate's
 * multi-tenant LLM gateway.
 *
 * ```ts
 * import { createLoomClient } from "@fieldstate/loom-client";
 *
 * const loom = createLoomClient({ baseUrl, apiKey });
 * const convo = loom.conversation({ model: "claude-haiku-4-5-20251001" });
 * convo.withMcp("lucidbrain").cached();
 * const message = await convo.send("Recall what we discussed about Titan.");
 * for await (const ev of convo.stream("And summarise it.")) {
 *   // ev is a normalised TurnEvent
 * }
 * ```
 */

export {
  createLoomClient,
  LoomClient,
  ConversationBuilder,
  toContent,
  LoomError,
  type TurnInput,
  type ConversationInit,
  type StatelessTurnInit,
} from "./client.js";

export type { TransportConfig } from "./http.js";
export type * from "./types.js";

// The generated OpenAPI surface is re-exported under a namespace so callers can
// reach the raw path/operation types when they want them.
export type { paths, components, operations } from "./generated.js";
