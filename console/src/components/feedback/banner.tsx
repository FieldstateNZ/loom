import type { CSSProperties, ReactNode } from "react";
import { Icon, type IconName } from "../core/icon.tsx";
import { IconButton } from "../core/icon-button.tsx";

/** The visual/semantic tone of a {@link Banner}, controlling its color and default icon. */
export type BannerTone = "info" | "ok" | "warn" | "danger";

/** Props for {@link Banner}. */
export interface BannerProps {
  /** Visual tone of the banner; defaults to "info". */
  readonly tone?: BannerTone;
  /** Overrides the tone's default icon. */
  readonly icon?: IconName;
  readonly title?: ReactNode;
  readonly action?: ReactNode;
  /** Called when the dismiss button is clicked; omit to hide the dismiss control. */
  readonly onDismiss?: () => void;
  readonly children?: ReactNode;
  readonly style?: CSSProperties;
}

const DEFAULT_ICONS: Record<BannerTone, IconName> = {
  info: "info",
  ok: "circle-check",
  warn: "triangle-alert",
  danger: "circle-alert",
};

/** Inline callout used to surface status, warnings, or errors near the content they relate to. */
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
