import type { CSSProperties, ReactNode } from "react";

/** The color/meaning conveyed by a {@link StatusDot}. */
export type StatusTone = "neutral" | "ok" | "warn" | "danger" | "accent";

/** Props for {@link StatusDot}. */
export interface StatusDotProps {
  readonly tone?: StatusTone;
  /** When true, animates the dot to draw attention (e.g. for an actively-in-progress state). */
  readonly pulse?: boolean;
  readonly label?: ReactNode;
  readonly style?: CSSProperties;
}

/** A small colored dot indicating status, optionally paired with a text label. */
export function StatusDot({ tone = "neutral", pulse = false, label, style }: StatusDotProps) {
  const dotCls = [
    "lm-statusdot__dot",
    tone !== "neutral" ? "lm-statusdot__dot--" + tone : "",
    pulse ? "lm-statusdot__dot--pulse" : "",
  ].filter(Boolean).join(" ");
  if (!label) return <span className={dotCls} style={style} />;
  return (
    <span className="lm-statusdot" style={style}>
      <span className={dotCls} />
      <span className="lm-statusdot__label">{label}</span>
    </span>
  );
}
