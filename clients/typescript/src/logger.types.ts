/**
 * {@link Logger} — the optional diagnostics sink a caller may inject.
 *
 * Library code must never reach for `console.*`: a consumer embedding this
 * client controls where logs go (or whether they exist at all). So the client
 * accepts an optional `Logger` and defaults to silence. It is only used for
 * genuinely diagnostic events (e.g. a malformed error body), never for control
 * flow — recoverable failures are returned as a `Result`, not logged.
 */

/**
 * A minimal structured logger. Every method is optional so a caller can supply
 * only the levels they care about; the client no-ops the rest.
 */
export interface Logger {
  /** Logs a warning — something unexpected but non-fatal happened. */
  readonly warn?: (message: string, meta?: unknown) => void;
  /** Logs a low-level diagnostic detail (per-request tracing, etc.). */
  readonly debug?: (message: string, meta?: unknown) => void;
}
