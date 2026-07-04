import type { CSSProperties } from "react";

export interface ChartSeries {
  name: string;
  color: string;
  data: number[];
}

export function ChartLegend({ series, style }: { series: ChartSeries[]; style?: CSSProperties }) {
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

const W = 600, H = 100, PAD_TOP = 6;

export interface LineChartProps {
  series?: ChartSeries[];
  height?: number;
  yFormat?: (v: number) => string | number;
  xLabels?: string[];
  area?: boolean;
  legend?: boolean;
  style?: CSSProperties;
}

export function LineChart({ series = [], height = 170, yFormat = (v) => v, xLabels = [], area = false, legend = true, style }: LineChartProps) {
  const all = series.flatMap((s) => s.data);
  const max = Math.max(...all, 0) || 1;
  const toPts = (data: number[]) => {
    const n = data.length;
    return data.map((v, i) => {
      const x = n === 1 ? W / 2 : (i / (n - 1)) * W;
      const y = H - ((v / max) * (H - PAD_TOP));
      return [x, y] as const;
    });
  };
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
          {series.map((s) => {
            const pts = toPts(s.data);
            const line = pts.map((p) => p[0].toFixed(1) + "," + p[1].toFixed(1)).join(" ");
            return (
              <g key={s.name}>
                {area ? (
                  <path d={"M0," + H + " L" + line.replace(/ /g, " L") + " L" + W + "," + H + " Z"} fill={s.color} opacity="0.09" stroke="none" />
                ) : null}
                <polyline points={line} fill="none" stroke={s.color} strokeWidth="1.5" vectorEffect="non-scaling-stroke" strokeLinejoin="round" />
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
