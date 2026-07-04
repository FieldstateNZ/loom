// Shapes usage-rollup rows into the console's bar-list and pivot models. Pure:
// no fetching, just sorting and field mapping over already-validated rows.
import { SERIES_COLORS, type UsageRow } from "./usage.ts";
import type { BarItem, UsageByKey } from "./metrics.ts";

/** Top-6 groups by cost as multi-colour bar rows (used for top models). */
export function topModelBars(rows: readonly UsageRow[]): BarItem[] {
  return sortByCost(rows)
    .slice(0, 6)
    .map((r, i) => ({
      label: r.group ?? "(unknown)",
      value: r.cost,
      display: `$${r.cost.toFixed(2)}`,
      color: SERIES_COLORS[i % SERIES_COLORS.length] ?? "var(--series-1)",
    }));
}

/** Top-6 groups by cost as single-colour bar rows (used for top keys). */
export function topKeyBars(rows: readonly UsageRow[]): BarItem[] {
  return sortByCost(rows)
    .slice(0, 6)
    .map((r) => ({ label: r.group ?? "(unknown)", value: r.cost, display: `$${r.cost.toFixed(2)}` }));
}

/** Per-key usage pivot rows from a key-grouped rollup. */
export function usageByKeyRows(rows: readonly UsageRow[]): UsageByKey[] {
  return rows.map((r) => ({
    key: r.group ?? "(unknown)",
    requests: r.event_count,
    input: r.input_tokens,
    output: r.output_tokens,
    cacheRead: r.cache_read_tokens,
    cacheWrite: r.cache_write_tokens,
    cost: r.cost,
  }));
}

/** Copies and sorts rows by descending cost (never mutates the input). */
function sortByCost(rows: readonly UsageRow[]): UsageRow[] {
  return rows.slice().sort((a, b) => b.cost - a.cost);
}
