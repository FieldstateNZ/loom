// Usage-rollup parsing and math. Both `/v1/usage` and `/admin/usage` return the
// same row shape; these functions validate the untrusted rows with Zod (money
// arrives as `rust_decimal` strings, so numerics are coerced) and derive the
// dashboard stat tiles from them.
import { z } from "zod";
import type { GatewayStats } from "./metrics.ts";

/** Series colours for top-N bar rows, cycled by index. */
export const SERIES_COLORS = [
  "var(--series-1)",
  "var(--series-2)",
  "var(--series-3)",
  "var(--series-4)",
] as const;

/** Coerces a JSON number/numeric-string/absent value to a finite number. */
const decimal = z
  .union([z.number(), z.string(), z.null(), z.undefined()])
  .transform((v) => {
    if (typeof v === "number") return Number.isFinite(v) ? v : 0;
    if (typeof v === "string") {
      const n = Number.parseFloat(v);
      return Number.isFinite(n) ? n : 0;
    }
    return 0;
  });

/** Schema for one usage-rollup row; the `UsageRow` type is derived from it. */
const UsageRowSchema = z.object({
  group: z
    .string()
    .nullish()
    .transform((v) => v ?? null),
  event_count: decimal,
  input_tokens: decimal,
  output_tokens: decimal,
  cache_read_tokens: decimal,
  cache_write_tokens: decimal,
  cost: decimal,
});

/** A validated usage-rollup row (a group's totals over a time window). */
export type UsageRow = z.infer<typeof UsageRowSchema>;

const ZERO_ROW: UsageRow = {
  group: null,
  event_count: 0,
  input_tokens: 0,
  output_tokens: 0,
  cache_read_tokens: 0,
  cache_write_tokens: 0,
  cost: 0,
};

const RowsSchema = z
  .object({ rows: z.array(z.unknown()).catch([]) })
  .catch({ rows: [] });

/** Extracts the row array from a usage response, tolerating any malformed shape. */
export function rowsOf(body: unknown): readonly unknown[] {
  return RowsSchema.parse(body).rows;
}

/** Validates one row, falling back to an all-zero row on any malformed input. */
export function toUsageRow(raw: unknown): UsageRow {
  return UsageRowSchema.catch(ZERO_ROW).parse(raw);
}

/** Aggregate totals across a set of rows. */
export interface RollupTotals {
  readonly cost: number;
  readonly input: number;
  readonly output: number;
  readonly cacheRead: number;
  readonly cacheWrite: number;
  readonly events: number;
}

/** Sums a set of usage rows into a single {@link RollupTotals}. */
export function totalRows(rows: readonly UsageRow[]): RollupTotals {
  return rows.reduce<RollupTotals>(
    (a, r) => ({
      cost: a.cost + r.cost,
      input: a.input + r.input_tokens,
      output: a.output + r.output_tokens,
      cacheRead: a.cacheRead + r.cache_read_tokens,
      cacheWrite: a.cacheWrite + r.cache_write_tokens,
      events: a.events + r.event_count,
    }),
    { cost: 0, input: 0, output: 0, cacheRead: 0, cacheWrite: 0, events: 0 },
  );
}

/** Rounded percentage change of `current` vs. `prior` (0 when there is no prior). */
export function pctDelta(current: number, prior: number): number {
  if (prior <= 0) return 0;
  return Math.round(((current - prior) / prior) * 100);
}

/** The all-zero stat tiles, used before any usage is available. */
export function emptyStats(): GatewayStats {
  return {
    spendToday: 0,
    spendPrior: 0,
    spendDelta: 0,
    tokensIn: 0,
    tokensInDelta: 0,
    tokensOut: 0,
    tokensOutDelta: 0,
    requests: 0,
    requestsDelta: 0,
    streams: 0,
    cacheReadToday: 0,
    cacheWriteToday: 0,
    cacheSavedToday: 0,
    cacheHitRate: 0,
  };
}

/** Derives the dashboard stat tiles from today's and the prior window's rows. */
export function statsFrom(
  todayRows: readonly UsageRow[],
  priorRows: readonly UsageRow[],
): GatewayStats {
  const t = totalRows(todayRows);
  const p = totalRows(priorRows);
  const cacheDenom = t.cacheRead + t.input;
  return {
    spendToday: t.cost,
    spendPrior: p.cost,
    spendDelta: pctDelta(t.cost, p.cost),
    tokensIn: t.input,
    tokensInDelta: pctDelta(t.input, p.input),
    tokensOut: t.output,
    tokensOutDelta: pctDelta(t.output, p.output),
    requests: t.events,
    requestsDelta: pctDelta(t.events, p.events),
    streams: 0, // no active-stream count endpoint
    cacheReadToday: t.cacheRead,
    cacheWriteToday: t.cacheWrite,
    cacheSavedToday: 0, // needs pricing the console cannot see
    cacheHitRate: cacheDenom > 0 ? t.cacheRead / cacheDenom : 0,
  };
}
