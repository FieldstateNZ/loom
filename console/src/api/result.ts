// The Result pattern — how the live client layer reports *expected* failures
// (a 4xx/5xx, an unreachable gateway, a missing credential, an unparseable
// body) without throwing. Callers branch on `result.ok` instead of wrapping
// every call in try/catch, which keeps error handling explicit and typed.

/** What went wrong. `kind` lets callers branch; `status` is set for HTTP failures. */
export interface LoomError {
  readonly kind: "http" | "network" | "config" | "parse";
  readonly message: string;
  readonly status?: number | undefined;
}

/** Either a successful `value` or a typed `error` — never both. */
export type Result<T, E = LoomError> =
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly error: E };

/** Wraps a success value. */
export function ok<T>(value: T): Result<T, never> {
  return { ok: true, value };
}

/** Wraps a failure. */
export function err<E>(error: E): Result<never, E> {
  return { ok: false, error };
}

/** Builds a {@link LoomError}, defaulting `kind` to an HTTP failure. */
export function loomError(
  message: string,
  kind: LoomError["kind"] = "http",
  status?: number,
): LoomError {
  return { kind, message, status };
}
