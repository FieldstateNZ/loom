/** The {@link ok} and {@link err} constructors for the {@link Result} union. */

import type { Result } from "./result.types.js";

/**
 * Wraps a success value in a {@link Result}.
 *
 * Use this at the point a fallible operation succeeds so the caller receives a
 * uniformly-shaped `{ ok: true, value }` they can narrow on.
 *
 * @param value - The successful value.
 */
export function ok<T>(value: T): Result<T, never> {
  return { ok: true, value };
}

/**
 * Wraps a failure in a {@link Result}.
 *
 * Use this instead of `throw` for any failure the gateway is expected to
 * report, so it surfaces in the return type and the caller cannot silently
 * ignore it.
 *
 * @param error - The error describing what went wrong.
 */
export function err<E>(error: E): Result<never, E> {
  return { ok: false, error };
}
