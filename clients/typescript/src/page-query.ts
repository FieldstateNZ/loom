/** Builds the `?limit=&offset=` query suffix for a paged history read. */

import type { PageParams } from "./page-query.types.js";

/**
 * Renders {@link PageParams} into a URL query suffix (including the leading
 * `?`), or an empty string when no paging was requested.
 *
 * Kept as a pure function so the two call sites (the builder and the top-level
 * client) share one encoding and stay consistent.
 *
 * @param page - The optional paging window.
 */
export function pageQuery(page?: PageParams): string {
  const query = new URLSearchParams();
  if (page?.limit !== undefined) query.set("limit", String(page.limit));
  if (page?.offset !== undefined) query.set("offset", String(page.offset));
  const rendered = query.toString();
  return rendered ? `?${rendered}` : "";
}
