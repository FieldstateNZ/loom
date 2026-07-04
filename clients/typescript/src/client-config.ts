/**
 * The client configuration and its Zod schema.
 *
 * `baseUrl` and `apiKey` come from outside the program (env vars, secrets), so
 * they are an external-input boundary and are validated with Zod before any
 * request is attempted. `fetch` and `logger` are injected dependencies, not
 * validated data, so they live on the type but not in the schema.
 */

import { z } from "zod";

import type { Logger } from "./logger.types.js";

/**
 * Validates the untrusted parts of the client config. Kept to the two fields
 * that can be malformed at runtime; the derived type is the single source of
 * truth for their shape.
 */
export const transportConfigSchema = z.object({
  baseUrl: z.string().url("baseUrl must be an absolute URL, e.g. http://127.0.0.1:8080"),
  apiKey: z.string().min(1, "apiKey must be a non-empty tenant key"),
});

/**
 * Configuration for a Loom client.
 *
 * The validated fields (`baseUrl`, `apiKey`) are derived from
 * {@link transportConfigSchema}; `fetch` and `logger` are optional injected
 * dependencies for tests, non-browser runtimes, and diagnostics.
 */
export type TransportConfig = Readonly<z.infer<typeof transportConfigSchema>> & {
  /** A `fetch` override (for tests or runtimes without a global `fetch`). */
  readonly fetch?: typeof fetch;
  /** An optional diagnostics sink. Defaults to silent. */
  readonly logger?: Logger;
};
