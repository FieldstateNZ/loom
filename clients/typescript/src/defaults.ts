/** Client-wide defaults applied when a caller does not specify otherwise. */

/**
 * The provider a conversation or turn binds to when `provider` is omitted.
 *
 * Loom is multi-provider, but Anthropic is the primary backend, so it is the
 * sensible default and keeps the common call site terse.
 */
export const DEFAULT_PROVIDER = "anthropic";
