/**
 * `@fieldstate/loom-client` — a TypeScript client for Loom, Fieldstate's
 * multi-tenant LLM gateway.
 *
 * Every fallible call returns a {@link Result} rather than throwing, so callers
 * branch on `ok` and read a structured {@link LoomError} on failure:
 *
 * ```ts
 * import { createLoomClient } from "@fieldstate/loom-client";
 *
 * const created = createLoomClient({ baseUrl, apiKey });
 * if (!created.ok) throw new Error(created.error.message);
 * const loom = created.value;
 *
 * const convo = loom.conversation({ model: "claude-haiku-4-5-20251001" });
 * convo.withMcp("lucidbrain").cached();
 *
 * const reply = await convo.send("Recall what we discussed about Titan.");
 * if (reply.ok) {
 *   for await (const ev of convo.stream("And summarise it.")) {
 *     if (ev.ok && ev.value.kind.type === "content_part_delta") {
 *       // render ev.value.kind.delta
 *     }
 *   }
 * }
 * ```
 */

// --- Client surface -------------------------------------------------------
export { createLoomClient } from "./create-loom-client.js";
export { LoomClient } from "./loom-client.js";
export { ConversationBuilder } from "./conversation-builder.js";
export { toContent } from "./to-content.js";
export { collect } from "./collect.js";

// --- Result pattern -------------------------------------------------------
export { ok, err } from "./result.js";
export type { Result } from "./result.types.js";

// --- Errors ---------------------------------------------------------------
export type {
  LoomError,
  LoomErrorCode,
  LoomHttpError,
  LoomNetworkError,
  LoomDecodeError,
  LoomStreamError,
  LoomConfigError,
} from "./loom-error.types.js";

// --- Config & input types -------------------------------------------------
export type { TransportConfig } from "./client-config.js";
export type { Logger } from "./logger.types.js";
export type { ConversationInit } from "./conversation-init.types.js";
export type { StatelessTurnInit } from "./stateless-turn.types.js";
export type { TurnInput } from "./to-content.types.js";
export type { PageParams } from "./page-query.types.js";
export type { CollectedTurn } from "./collect.types.js";

// --- Domain models --------------------------------------------------------
export type * from "./models/index.js";

// The generated OpenAPI surface is re-exported so callers can reach the raw
// path/operation types when they want them.
export type { paths, components, operations } from "./generated.js";
