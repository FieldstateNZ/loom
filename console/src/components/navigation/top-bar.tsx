import { Fragment, type CSSProperties, type ReactNode } from "react";
import { Icon } from "../core/icon.tsx";

/** A single breadcrumb segment in a {@link TopBar}; clickable unless it's the current (last) crumb. */
export interface Crumb {
  readonly label: string;
  readonly onClick?: () => void;
}

/** Props for {@link TopBar}. */
export interface TopBarProps {
  /** Breadcrumb trail; plain strings are treated as non-clickable labels. */
  readonly crumbs?: (string | Crumb)[];
  readonly actions?: ReactNode;
  readonly style?: CSSProperties;
}

/** Page header showing a breadcrumb trail with optional trailing action buttons. */
export function TopBar({ crumbs = [], actions, style }: TopBarProps) {
  return (
    <header className="lm-topbar" style={style}>
      <div className="lm-topbar__crumbs">
        {crumbs.map((c, i) => {
          const crumb: Crumb = typeof c === "string" ? { label: c } : c;
          const last = i === crumbs.length - 1;
          return (
            <Fragment key={i}>
              {i > 0 ? <span className="lm-topbar__sep"><Icon name="chevron-right" size={12} /></span> : null}
              {last ? (
                <span className="lm-topbar__crumb lm-topbar__crumb--current">{crumb.label}</span>
              ) : (
                <button type="button" className="lm-topbar__crumb" onClick={crumb.onClick}>{crumb.label}</button>
              )}
            </Fragment>
          );
        })}
      </div>
      {actions ? <div className="lm-topbar__actions">{actions}</div> : null}
    </header>
  );
}
