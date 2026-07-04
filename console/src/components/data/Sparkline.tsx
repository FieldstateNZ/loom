import type { CSSProperties } from "react";

export interface SparklineProps {
  data?: number[];
  width?: number | string;
  height?: number | string;
  color?: string;
  fill?: boolean;
  strokeWidth?: number;
  style?: CSSProperties;
}

export function Sparkline({ data = [], width = 120, height = 28, color = "var(--accent)", fill = true, strokeWidth = 1.5, style }: SparklineProps) {
  const W = 100, H = 100;
  const n = data.length;
  if (n < 2) return <svg width={width} height={height} style={style} aria-hidden="true" />;
  const min = Math.min(...data), max = Math.max(...data);
  const span = max - min || 1;
  const padY = 8;
  const pts = data.map((v, i) => {
    const x = (i / (n - 1)) * W;
    const y = H - padY - ((v - min) / span) * (H - padY * 2);
    return [x, y] as const;
  });
  const line = pts.map((p) => p[0].toFixed(2) + "," + p[1].toFixed(2)).join(" ");
  const area = "M0," + H + " L" + line.replace(/ /g, " L") + " L" + W + "," + H + " Z";
  return (
    <svg
      width={width} height={height} viewBox={"0 0 " + W + " " + H}
      preserveAspectRatio="none" aria-hidden="true"
      style={{ display: "block", overflow: "visible", ...style }}
    >
      {fill ? <path d={area} fill={color} opacity="0.1" stroke="none" /> : null}
      <polyline points={line} fill="none" stroke={color} strokeWidth={strokeWidth} vectorEffect="non-scaling-stroke" strokeLinejoin="round" strokeLinecap="round" />
    </svg>
  );
}
