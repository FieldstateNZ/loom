/**
 * The {@link Result} type — this client's contract for "expected" failures.
 *
 * Loom's HTTP surface fails in routine, recoverable ways: a tenant key is
 * rejected (401), a conversation id is unknown (404), a budget is exhausted
 * (402), the upstream provider hiccups (502). Those are *not* bugs — they are
 * normal outcomes a caller must branch on. Rather than model them as thrown
 * exceptions (which are invisible in the type signature and easy to forget to
 * catch), every fallible operation returns a `Result`: a tagged union that the
 * type-checker forces the caller to inspect before reaching the value.
 *
 * Throwing is reserved for genuine programmer error (misuse of the API), never
 * for a failure the gateway is expected to report.
 */

/**
 * The outcome of a fallible operation: either a success carrying a `value`, or
 * a failure carrying an `error`. The `ok` discriminant is what you branch on.
 *
 * @typeParam T - The value produced on success.
 * @typeParam E - The error produced on failure.
 */
export type Result<T, E> =
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly error: E };
