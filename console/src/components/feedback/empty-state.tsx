import type { CSSProperties, ReactNode } from "react";
import { Icon, type IconName } from "../core/icon.tsx";

export interface EmptyStateProps {
  icon?: IconName;
  title?: ReactNode;
  hint?: ReactNode;
  action?: ReactNode;
  style?: CSSProperties;
}

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
