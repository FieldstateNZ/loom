import type { CSSProperties } from "react";
import type { ChartSeries } from "./chart-series.ts";

/** Props for {@link ChartLegend}. */
export interface ChartLegendProps {
  readonly series: readonly ChartSeries[];
  readonly style?: CSSProperties;
}

/** A compact swatch + name legend, shared by the line and bar charts. */
export function ChartLegend({ series, style }: ChartLegendProps) {
  return (
    <div style={{ display: "flex", gap: "14px", flexWrap: "wrap", ...style }}>
      {series.map((s) => (
        <span key={s.name} style={{ display: "inline-flex", alignItems: "center", gap: "6px", font: "var(--w-reg) var(--fs-11) / 1 var(--font-mono)", color: "var(--fg-3)" }}>
          <span style={{ width: "8px", height: "2px", borderRadius: "1px", background: s.color, flexShrink: 0 }}></span>
          {s.name}
        </span>
      ))}
    </div>
  );
}
