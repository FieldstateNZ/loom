/**
 * {@link ServerTool} — a provider-executed (server-side) tool offered on a turn.
 *
 * Request-only (the caller offers it), so it is a plain readonly union tagged on
 * `kind`. The `raw` arm is an escape hatch for provider tools the client does
 * not model yet: any extra fields pass through verbatim.
 */

/** A provider-executed (server-side) tool (internally tagged on `kind`). */
export type ServerTool =
  | {
      readonly kind: "web_search";
      readonly max_uses?: number;
      readonly allowed_domains?: readonly string[];
      readonly blocked_domains?: readonly string[];
    }
  | { readonly kind: "code_execution" }
  | ({ readonly kind: "raw" } & { readonly [key: string]: unknown });
