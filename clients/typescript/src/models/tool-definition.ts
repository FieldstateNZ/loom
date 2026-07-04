/**
 * {@link ToolDefinition} — a client-side tool the model may call.
 *
 * Request-only (the caller declares it), so it is a plain readonly type. The
 * `input_schema` is the tool's JSON Schema; it is `unknown` because its shape is
 * the caller's to define, not the gateway's to constrain.
 */

import type { CacheHint } from "./cache.js";

/** The definition of a client-side tool the model may call. */
export interface ToolDefinition {
  /** The tool name the model uses to invoke it. */
  readonly name: string;
  /** A natural-language description guiding when the model should call it. */
  readonly description?: string;
  /** The tool's input JSON Schema (caller-defined shape). */
  readonly input_schema: unknown;
  /** An optional prompt-cache breakpoint for this definition. */
  readonly cache?: CacheHint;
}
