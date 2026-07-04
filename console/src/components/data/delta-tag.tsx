import type { CSSProperties } from "react";
import { Icon } from "../core/icon.tsx";

/** Props for {@link DeltaTag}. */
export interface DeltaTagProps {
  readonly value: number | null | undefined;
  /** When true, a decrease is shown as good and an increase as bad (e.g. for cost metrics). */
  readonly invert?: boolean;
  readonly suffix?: string;
  readonly style?: CSSProperties;
}

/** Displays a signed change value (e.g. "+12%") colored green or red to indicate whether it's a good or bad trend. */
export function DeltaTag({ value, invert = false, suffix = "%", style }: DeltaTagProps) {
  const flat = value === 0 || value == null;
  const up = (value || 0) > 0;
  const good = flat ? null : invert ? !up : up;
  const color = flat ? "var(--fg-3)" : good ? "var(--ok)" : "var(--danger)";
  const text = flat ? "±0" + suffix : (up ? "+" : "−") + Math.abs(value as number) + suffix;
  return (
    <span
      style={{
        display: "inline-flex", alignItems: "center", gap: "2px",
        font: "var(--w-med) var(--fs-11) / 1 var(--font-mono)",
        color, whiteSpace: "nowrap", ...style,
      }}
      title={invert ? "vs prior period (up is worse)" : "vs prior period"}
    >
      {!flat ? <Icon name={up ? "arrow-up-right" : "arrow-down-right"} size={11} /> : null}
      {text}
    </span>
  );
}
