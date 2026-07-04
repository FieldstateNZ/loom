import type { CSSProperties, ReactNode } from "react";
import { Icon, type IconName } from "./icon.tsx";

/** The visual tone/color treatment a {@link Badge} can be rendered in. */
export type BadgeTone = "neutral" | "ok" | "warn" | "danger" | "info" | "accent";

/** Props for {@link Badge}. */
export interface BadgeProps {
  readonly tone?: BadgeTone;
  readonly caps?: boolean;
  readonly icon?: IconName | undefined;
  readonly children?: ReactNode;
  readonly style?: CSSProperties;
}

/** A small pill-shaped label used to call out a status, category, or other short piece of metadata. */
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
