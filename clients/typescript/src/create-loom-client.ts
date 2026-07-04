/** The {@link createLoomClient} factory — the package's main entry point. */

import { transportConfigSchema } from "./client-config.js";
import type { TransportConfig } from "./client-config.js";
import { LoomClient } from "./loom-client.js";
import { configError, zodIssues } from "./loom-error.js";
import type { LoomError } from "./loom-error.types.js";
import { err, ok } from "./result.js";
import type { Result } from "./result.types.js";
import { Transport } from "./transport.js";

/**
 * Creates a {@link LoomClient}, validating the config first.
 *
 * The config's `baseUrl`/`apiKey` are an external-input boundary, so they are
 * validated with Zod: a malformed config returns a `Result` failure rather than
 * throwing, so a caller can surface it like any other error. The client itself
 * is only constructed once the config is known-good.
 *
 * (It does throw in one case: a runtime with no global `fetch` and no injected
 * `fetch` — that is an environment/programmer error, not a gateway failure.)
 *
 * @param config - The base URL, tenant key, and optional `fetch`/`logger`.
 * @returns A ready client, or a {@link LoomError} describing the invalid config.
 */
export function createLoomClient(config: TransportConfig): Result<LoomClient, LoomError> {
  const parsed = transportConfigSchema.safeParse(config);
  if (!parsed.success) {
    return err(configError(zodIssues(parsed.error)));
  }
  return ok(new LoomClient(new Transport(config)));
}
