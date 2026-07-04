/**
 * {@link DeepReadonly} — recursively marks every property of a type `readonly`.
 *
 * Response DTOs are inferred from Zod schemas (`z.infer`), and Zod produces
 * mutable types. This client never mutates a decoded response, and the codebase
 * rule is "`readonly` everywhere it makes sense", so we wrap each inferred
 * response type in `DeepReadonly` to add the modifiers without hand-writing a
 * second copy of the shape beside the schema.
 */

/**
 * Recursively applies `readonly` to arrays and object properties of `T`.
 * Primitives, `unknown`, and functions pass through unchanged.
 *
 * @typeParam T - The type to freeze at the type level.
 */
export type DeepReadonly<T> = T extends (infer U)[]
  ? ReadonlyArray<DeepReadonly<U>>
  : T extends readonly (infer U)[]
    ? ReadonlyArray<DeepReadonly<U>>
    : T extends object
      ? { readonly [K in keyof T]: DeepReadonly<T[K]> }
      : T;
