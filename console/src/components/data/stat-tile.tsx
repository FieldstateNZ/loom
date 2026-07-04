import type { CSSProperties, ReactNode } from "react";
import { Sparkline } from "./sparkline.tsx";
import { DeltaTag } from "./delta-tag.tsx";

/** Props for {@link StatTile}. */
export interface StatTileProps {
  readonly label: ReactNode;
  readonly labelRight?: ReactNode;
  readonly value: ReactNode;
  readonly delta?: number | null;
  /** When true, a decrease is shown as good and an increase as bad (e.g. for cost metrics). */
  readonly invertDelta?: boolean;
  readonly sub?: ReactNode;
  readonly spark?: readonly number[];
  readonly sparkColor?: string | undefined;
  /** Renders the tile in a larger, emphasized style for headline metrics. */
  readonly hero?: boolean;
  readonly style?: CSSProperties;
}

/** Displays a single labeled metric with an optional delta and sparkline, used to summarize a key figure. */
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
