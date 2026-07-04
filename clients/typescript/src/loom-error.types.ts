/**
 * {@link LoomError} — the structured error every fallible client call can fail
 * with, modelled as a discriminated union so callers can branch precisely.
 *
 * The gateway renders all HTTP failures as a stable envelope
 * `{ "error": { "code", "message", "provider_error"?, "details"? } }` (see
 * `crates/loom-server/src/error.rs`). We surface that envelope faithfully, and
 * additionally distinguish failures that never reach the gateway (network) or
 * that arrive malformed (decode), plus client-side config validation. The
 * `kind` field is the discriminant.
 */

/**
 * The gateway's stable, machine-readable error codes. Prefer branching on these
 * over parsing `message` prose. Kept open-ended (`| (string & {})`) because the
 * server may add codes the client predates; unknown codes still type-check.
 */
export type LoomErrorCode =
  | "unauthorized"
  | "bad_request"
  | "not_found"
  | "conflict"
  | "unavailable"
  | "budget_exceeded"
  | "rate_limited"
  | "capability_unsupported"
  | "model_not_found"
  | "provider_error"
  | "provider_unavailable"
  | "internal"
  // `string & {}` keeps the known codes as editor hints while still accepting
  // future server codes verbatim.
  | (string & {});

/**
 * A structured error the gateway returned over HTTP (any non-2xx response whose
 * body was the standard envelope). This is the common failure a caller handles.
 */
export interface LoomHttpError {
  readonly kind: "http";
  /** The HTTP status code (e.g. `404`, `402`, `502`). */
  readonly status: number;
  /** The stable machine-readable code from `error.code`. */
  readonly code: LoomErrorCode;
  /** The human-readable, non-sensitive message from `error.message`. */
  readonly message: string;
  /** The provider's own verbatim error payload, when the gateway forwarded one. */
  readonly providerError?: unknown;
  /** Structured extras from `error.details` (e.g. the budget breakdown). */
  readonly details?: unknown;
}

/**
 * The request never produced an HTTP response — `fetch` itself rejected
 * (connection refused, DNS failure, TLS error, abort). There is no status.
 */
export interface LoomNetworkError {
  readonly kind: "network";
  /** A human-readable description of the transport failure. */
  readonly message: string;
  /** The underlying thrown value, preserved for logging/debugging. */
  readonly cause?: unknown;
}

/**
 * A response arrived but its body could not be parsed as JSON, or did not match
 * the Zod schema for the endpoint. Signals a gateway/client contract mismatch.
 */
export interface LoomDecodeError {
  readonly kind: "decode";
  /** A human-readable description of what failed to decode. */
  readonly message: string;
  /** Zod issue messages (or similar), when validation produced them. */
  readonly issues?: readonly string[];
}

/**
 * A terminal `error` frame arrived mid-stream on an SSE turn. The turn began
 * successfully (HTTP 200) but the provider or gateway aborted it; the envelope
 * is carried through so the caller can inspect `code`/`message`.
 */
export interface LoomStreamError {
  readonly kind: "stream";
  /** The stable machine-readable code from the frame's envelope, if present. */
  readonly code: LoomErrorCode;
  /** The human-readable message from the frame's envelope. */
  readonly message: string;
  /** The provider's own verbatim error payload, when present. */
  readonly providerError?: unknown;
}

/**
 * The client config (`baseUrl` / `apiKey`) failed validation before any request
 * was attempted. Returned by {@link createLoomClient}.
 */
export interface LoomConfigError {
  readonly kind: "config";
  /** A human-readable description of the invalid config. */
  readonly message: string;
  /** The individual field-level validation messages. */
  readonly issues: readonly string[];
}

/**
 * Any failure a fallible client operation can return. Discriminate on `kind`
 * (`"http" | "network" | "decode" | "stream" | "config"`) then read the
 * variant-specific fields.
 */
export type LoomError =
  | LoomHttpError
  | LoomNetworkError
  | LoomDecodeError
  | LoomStreamError
  | LoomConfigError;
