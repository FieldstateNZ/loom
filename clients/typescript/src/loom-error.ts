/**
 * Pure constructors that turn raw failure inputs (an error response body, a
 * thrown `fetch`, a bad config) into a structured {@link LoomError}.
 *
 * These are the single place the gateway's error envelope is parsed, so the
 * transport and stream layers never hand-shape an error object.
 */

import { z } from "zod";

import type {
  LoomConfigError,
  LoomDecodeError,
  LoomHttpError,
  LoomNetworkError,
  LoomStreamError,
} from "./loom-error.types.js";

/**
 * The gateway's error envelope: `{ error: { code, message, provider_error?,
 * details? } }`. Untrusted input, so it is validated rather than cast.
 */
const errorEnvelopeSchema = z.object({
  error: z.object({
    code: z.string(),
    message: z.string(),
    provider_error: z.unknown().optional(),
    details: z.unknown().optional(),
  }),
});

/**
 * Parses a non-2xx HTTP response body into a {@link LoomHttpError}.
 *
 * Falls back gracefully when the body is empty or not the standard envelope
 * (e.g. a plain-text error injected by an upstream proxy), so a caller always
 * gets a status and a best-effort message rather than a decode failure.
 *
 * @param status - The HTTP status code of the response.
 * @param statusText - The HTTP status text, used as a last-resort message.
 * @param bodyText - The raw response body text (may be empty).
 */
export function parseErrorEnvelope(
  status: number,
  statusText: string,
  bodyText: string,
): LoomHttpError {
  const fallback = statusText || `HTTP ${status}`;
  if (!bodyText) {
    return { kind: "http", status, code: "unknown", message: fallback };
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(bodyText);
  } catch {
    return { kind: "http", status, code: "unknown", message: bodyText };
  }
  const envelope = errorEnvelopeSchema.safeParse(parsed);
  if (envelope.success) {
    const { code, message, provider_error, details } = envelope.data.error;
    return {
      kind: "http",
      status,
      code,
      message,
      ...(provider_error !== undefined ? { providerError: provider_error } : {}),
      ...(details !== undefined ? { details } : {}),
    };
  }
  return { kind: "http", status, code: "unknown", message: fallback };
}

/**
 * Builds a {@link LoomStreamError} from a decoded terminal SSE `error` frame.
 *
 * @param payload - The parsed JSON of the frame's `data:` field.
 */
export function streamError(payload: unknown): LoomStreamError {
  const envelope = errorEnvelopeSchema.safeParse(payload);
  if (envelope.success) {
    const { code, message, provider_error } = envelope.data.error;
    return {
      kind: "stream",
      code,
      message,
      ...(provider_error !== undefined ? { providerError: provider_error } : {}),
    };
  }
  return { kind: "stream", code: "unknown", message: "stream error" };
}

/**
 * Builds a {@link LoomNetworkError} for a `fetch` that never produced a
 * response (connection refused, DNS failure, TLS error, abort).
 *
 * @param cause - The value `fetch` rejected with.
 */
export function networkError(cause: unknown): LoomNetworkError {
  const message = cause instanceof Error ? cause.message : "network request failed";
  return { kind: "network", message, cause };
}

/**
 * Builds a {@link LoomDecodeError} for a 2xx body that failed JSON parsing or
 * Zod validation.
 *
 * @param message - What could not be decoded.
 * @param issues - Optional field-level validation messages.
 */
export function decodeError(message: string, issues?: readonly string[]): LoomDecodeError {
  return { kind: "decode", message, ...(issues ? { issues } : {}) };
}

/**
 * Builds a {@link LoomConfigError} for an invalid client config, before any
 * request is attempted.
 *
 * @param issues - The field-level validation messages.
 */
export function configError(issues: readonly string[]): LoomConfigError {
  return { kind: "config", message: "invalid Loom client config", issues };
}

/**
 * Flattens a Zod validation error into human-readable `path: message` strings,
 * suitable for a {@link LoomDecodeError} or {@link LoomConfigError}'s `issues`.
 *
 * @param error - The Zod error to flatten.
 */
export function zodIssues(error: z.ZodError): readonly string[] {
  return error.issues.map((issue) => `${issue.path.join(".") || "(root)"}: ${issue.message}`);
}
