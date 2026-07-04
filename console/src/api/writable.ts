/**
 * Strips `readonly` from every property of `T`. Used only where the live client
 * assembles a public readonly DTO field-by-field from streamed JSON, before
 * handing back the finished (readonly) value.
 */
export type Writable<T> = { -readonly [K in keyof T]: T[K] };
