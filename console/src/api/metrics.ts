// Aggregate usage/spend metrics — the numbers the dashboard and usage explorer
// render. All `readonly`: computed once per snapshot, never mutated by the UI.

/** The dashboard stat-tile figures, today vs. the prior comparable window. */
export interface GatewayStats {
  readonly spendToday: number;
  readonly spendPrior: number;
  /** Percentage change vs. prior, already rounded (e.g. `23` = +23%). */
  readonly spendDelta: number;
  readonly tokensIn: number;
  readonly tokensInDelta: number;
  readonly tokensOut: number;
  readonly tokensOutDelta: number;
  readonly requests: number;
  readonly requestsDelta: number;
  readonly streams: number;
  readonly cacheReadToday: number;
  readonly cacheWriteToday: number;
  readonly cacheSavedToday: number;
  /** Fraction of input tokens served from cache, 0..1. */
  readonly cacheHitRate: number;
}

/** Day-bucketed usage series for the usage explorer's charts (parallel arrays). */
export interface UsageDaily {
  readonly labels: readonly string[];
  readonly cost: readonly number[];
  readonly input: readonly number[];
  readonly output: readonly number[];
  readonly cacheRead: readonly number[];
  readonly cacheWrite: readonly number[];
}

/** One row in a horizontal bar list (top models, top keys). */
export interface BarItem {
  readonly label: string;
  readonly value: number;
  /** Pre-formatted value label (e.g. `"$8.90"`); falls back to `value`. */
  readonly display?: string;
  readonly color?: string;
  readonly key?: string;
}

/** Severity tone for a gateway event row. */
export type EventTone = "danger" | "warn";

/** A recent block/error/warn event for the dashboard feed. */
export interface GatewayEvent {
  readonly time: string;
  readonly kind: "block" | "error" | "warn";
  readonly tone: EventTone;
  readonly key: string;
  readonly detail: string;
}

/** A per-key usage pivot row for the usage explorer table. */
export interface UsageByKey {
  readonly key: string;
  readonly requests: number;
  readonly input: number;
  readonly output: number;
  readonly cacheRead: number;
  readonly cacheWrite: number;
  readonly cost: number;
}
