import type { CSSProperties, ReactNode } from "react";
import { Sparkline } from "./Sparkline.tsx";
import { DeltaTag } from "./DeltaTag.tsx";

export interface StatTileProps {
  label: ReactNode;
  labelRight?: ReactNode;
  value: ReactNode;
  delta?: number | null;
  invertDelta?: boolean;
  sub?: ReactNode;
  spark?: number[];
  sparkColor?: string;
  hero?: boolean;
  style?: CSSProperties;
}

export function StatTile({ label, labelRight, value, delta, invertDelta = false, sub, spark, sparkColor, hero = false, style }: StatTileProps) {
  return (
    <div className={"lm-stat" + (hero ? " lm-stat--hero" : "")} style={style}>
      <div className="lm-stat__label">
        <span>{label}</span>
        {labelRight ? <span>{labelRight}</span> : null}
      </div>
      <div className="lm-stat__value-row">
        <span className="lm-stat__value">{value}</span>
        {delta != null ? <DeltaTag value={delta} invert={invertDelta} /> : null}
      </div>
      {sub ? <div className="lm-stat__sub">{sub}</div> : null}
      {spark ? (
        <div className="lm-stat__spark">
          <Sparkline data={spark} width="100%" height={26} color={sparkColor} />
        </div>
      ) : null}
    </div>
  );
}
