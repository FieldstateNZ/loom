import type { CSSProperties, ReactNode } from "react";
import { Icon, type IconName } from "./Icon.tsx";

export type BadgeTone = "neutral" | "ok" | "warn" | "danger" | "info" | "accent";

export interface BadgeProps {
  tone?: BadgeTone;
  caps?: boolean;
  icon?: IconName;
  children?: ReactNode;
  style?: CSSProperties;
}

export function Badge({ tone = "neutral", caps = false, icon, children, style }: BadgeProps) {
  const cls = [
    "lm-badge",
    tone !== "neutral" ? "lm-badge--" + tone : "",
    caps ? "lm-badge--caps" : "",
  ].filter(Boolean).join(" ");
  return (
    <span className={cls} style={style}>
      {icon ? <Icon name={icon} size={11} /> : null}
      {children}
    </span>
  );
}
