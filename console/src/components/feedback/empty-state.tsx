import type { CSSProperties, ReactNode } from "react";
import { Icon, type IconName } from "../core/icon.tsx";

/** Props for {@link EmptyState}. */
export interface EmptyStateProps {
  readonly icon?: IconName;
  readonly title?: ReactNode;
  readonly hint?: ReactNode;
  readonly action?: ReactNode;
  readonly style?: CSSProperties;
}

/** Placeholder shown in place of a list/panel when there is no data to display yet. */
export function EmptyState({ icon = "layers", title, hint, action, style }: EmptyStateProps) {
  return (
    <div className="lm-empty" style={style}>
      <div className="lm-empty__icon"><Icon name={icon} size={17} /></div>
      {title ? <p className="lm-empty__title">{title}</p> : null}
      {hint ? <p className="lm-empty__hint">{hint}</p> : null}
      {action ? <div className="lm-empty__action">{action}</div> : null}
    </div>
  );
}
