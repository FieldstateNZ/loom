import type { CSSProperties, ReactNode } from "react";

export type StatusTone = "neutral" | "ok" | "warn" | "danger" | "accent";

export interface StatusDotProps {
  tone?: StatusTone;
  pulse?: boolean;
  label?: ReactNode;
  style?: CSSProperties;
}

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
