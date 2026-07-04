import type { CSSProperties } from "react";
import { ChartLegend, type ChartSeries } from "./LineChart.tsx";

const W = 600, H = 100, PAD_TOP = 6;

export interface BarChartProps {
  series?: ChartSeries[];
  height?: number;
  yFormat?: (v: number) => string | number;
  xLabels?: string[];
  legend?: boolean;
  titles?: string[];
  style?: CSSProperties;
}

export function BarChart({ series = [], height = 170, yFormat = (v) => v, xLabels = [], legend = true, titles, style }: BarChartProps) {
  const n = Math.max(...series.map((s) => s.data.length), 0);
  const totals = Array.from({ length: n }, (_, i) => series.reduce((sum, s) => sum + (s.data[i] || 0), 0));
  const max = Math.max(...totals, 0) || 1;
  const slot = W / (n || 1);
  const barW = Math.min(slot * 0.6, 34);
  const gridYs = [0.25, 0.5, 0.75, 1];
  return (
    <div style={{ minWidth: 0, ...style }}>
      {legend && series.length > 1 ? <ChartLegend series={series} style={{ justifyContent: "flex-end", marginBottom: "10px" }} /> : null}
      <div style={{ position: "relative" }}>
        <svg width="100%" height={height} viewBox={"0 0 " + W + " " + H} preserveAspectRatio="none" style={{ display: "block" }} aria-hidden="true">
          {gridYs.map((g) => (
            <line key={g} x1="0" x2={W} y1={H - g * (H - PAD_TOP)} y2={H - g * (H - PAD_TOP)} stroke="var(--border-1)" strokeWidth="1" vectorEffect="non-scaling-stroke" strokeDasharray={g === 1 ? undefined : "1 4"} />
          ))}
          <line x1="0" x2={W} y1={H} y2={H} stroke="var(--border-2)" strokeWidth="1" vectorEffect="non-scaling-stroke" />
          {Array.from({ length: n }, (_, i) => {
            let acc = 0;
            const x = i * slot + (slot - barW) / 2;
            return (
              <g key={i}>
                {titles ? <title>{titles[i]}</title> : null}
                {series.map((s) => {
                  const v = s.data[i] || 0;
                  const h = (v / max) * (H - PAD_TOP);
                  acc += h;
                  return <rect key={s.name} x={x} y={H - acc} width={barW} height={Math.max(h - 0.5, 0)} fill={s.color} />;
                })}
              </g>
            );
          })}
        </svg>
        <span style={{ position: "absolute", top: "-4px", left: 0, font: "var(--w-reg) 10px / 1 var(--font-mono)", color: "var(--fg-4)" }}>{yFormat(max)}</span>
        <span style={{ position: "absolute", top: "calc(50% - 8px)", left: 0, font: "var(--w-reg) 10px / 1 var(--font-mono)", color: "var(--fg-4)" }}>{yFormat(max / 2)}</span>
      </div>
      {xLabels.length ? (
        <div style={{ display: "flex", justifyContent: "space-between", marginTop: "6px", font: "var(--w-reg) 10px / 1 var(--font-mono)", color: "var(--fg-4)" }}>
          {xLabels.map((l, i) => <span key={i}>{l}</span>)}
        </div>
      ) : null}
    </div>
  );
}
