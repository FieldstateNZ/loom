// Small helpers for narrowing untrusted JSON. The gateway's payloads arrive as
// `unknown`; these coerce individual fields defensively so a missing or
// malformed value degrades to a sensible default rather than throwing.

/**
 * Coerces a JSON number-or-numeric-string to a finite number, defaulting to 0.
 * The gateway serializes `rust_decimal` money values as strings, so numeric
 * fields must accept both forms.
 */
export function num(v: unknown): number {
  if (typeof v === "number") return Number.isFinite(v) ? v : 0;
  if (typeof v === "string") {
    const n = Number.parseFloat(v);
    return Number.isFinite(n) ? n : 0;
  }
  return 0;
}

/** Narrows an unknown JSON value to a plain record, or `null` if it is not one. */
export function asRecord(v: unknown): Record<string, unknown> | null {
  return v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
}

/** Returns the value if it is a string, else `undefined`. */
export function str(v: unknown): string | undefined {
  return typeof v === "string" ? v : undefined;
}
