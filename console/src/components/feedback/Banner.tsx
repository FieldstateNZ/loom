import type { CSSProperties, ReactNode } from "react";
import { Icon, type IconName } from "../core/Icon.tsx";
import { IconButton } from "../core/IconButton.tsx";

export type BannerTone = "info" | "ok" | "warn" | "danger";

export interface BannerProps {
  tone?: BannerTone;
  icon?: IconName;
  title?: ReactNode;
  action?: ReactNode;
  onDismiss?: () => void;
  children?: ReactNode;
  style?: CSSProperties;
}

const DEFAULT_ICONS: Record<BannerTone, IconName> = {
  info: "info",
  ok: "circle-check",
  warn: "triangle-alert",
  danger: "circle-alert",
};

export function Banner({ tone = "info", icon, title, action, onDismiss, children, style }: BannerProps) {
  return (
    <div className={"lm-banner lm-banner--" + tone} role={tone === "danger" || tone === "warn" ? "alert" : "status"} style={style}>
      <span className="lm-banner__icon"><Icon name={icon || DEFAULT_ICONS[tone]} size={15} /></span>
      <div className="lm-banner__content">
        {title ? <p className="lm-banner__title">{title}</p> : null}
        {children}
      </div>
      {(action || onDismiss) ? (
        <span className="lm-banner__side">
          {action}
          {onDismiss ? <IconButton icon="x" label="Dismiss" size="sm" onClick={onDismiss} /> : null}
        </span>
      ) : null}
    </div>
  );
}
