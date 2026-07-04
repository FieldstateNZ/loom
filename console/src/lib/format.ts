// Num formatters — the console's number renderer helpers. All data numerals are
// mono + tabular. Ported verbatim from the design bundle's data/Num.jsx. Each
// formatter treats null/undefined/NaN as an em-dash so callers never guard.

/** Options for {@link formatMoney}. */
export interface MoneyOpts {
  /** Abbreviate large values (e.g. `$1.2M`, `$4.1K`) for tight chart axes. */
  readonly compact?: boolean;
}

/** Formats a USD amount; small values keep extra precision, negatives use a real minus. */
export function formatMoney(v: number | null | undefined, opts: MoneyOpts = {}): string {
  if (v == null || isNaN(v)) return "—";
  const neg = v < 0 ? "−" : "";
  const a = Math.abs(v);
  if (opts.compact && a >= 1e6) return neg + "$" + (a / 1e6).toFixed(1) + "M";
  if (opts.compact && a >= 1e4) return neg + "$" + (a / 1e3).toFixed(1) + "K";
  if (a !== 0 && a < 0.01) return neg + "$" + a.toFixed(4);
  if (a !== 0 && a < 1) return neg + "$" + a.toFixed(3);
  return neg + "$" + a.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
}

/** Formats a token/row count with a compact B/M/K suffix above 10k. */
export function formatTokens(v: number | null | undefined): string {
  if (v == null || isNaN(v)) return "—";
  const a = Math.abs(v);
  if (a >= 1e9) return (v / 1e9).toFixed(1) + "B";
  if (a >= 1e6) return (v / 1e6).toFixed(1) + "M";
  if (a >= 1e5) return Math.round(v / 1e3) + "K";
  if (a >= 1e4) return (v / 1e3).toFixed(1) + "K";
  return v.toLocaleString("en-US");
}

/** Formats a generic count (alias of {@link formatTokens} for readable call sites). */
export function formatCount(v: number | null | undefined): string {
  return formatTokens(v);
}

/** Formats a duration in milliseconds as `ms` / `s` / `m s`. */
export function formatMs(v: number | null | undefined): string {
  if (v == null || isNaN(v)) return "—";
  if (v < 1000) return Math.round(v) + "ms";
  if (v < 60000) return (v / 1000).toFixed(1) + "s";
  return Math.floor(v / 60000) + "m " + Math.round((v % 60000) / 1000) + "s";
}

/** Formats a 0..1 ratio as a percentage, keeping one decimal below 10%. */
export function formatPercent(v: number | null | undefined): string {
  if (v == null || isNaN(v)) return "—";
  const pct = v * 100;
  return (Math.abs(pct) < 10 && pct !== 0 ? pct.toFixed(1) : Math.round(pct)) + "%";
}

/** Formatter bundle — screens reach these as Fmt.money etc. */
export const Fmt = {
  money: formatMoney,
  tokens: formatTokens,
  count: formatCount,
  ms: formatMs,
  percent: formatPercent,
};
