import { Fragment, type CSSProperties, type ReactNode } from "react";
import { Icon, type IconName } from "../core/icon.tsx";

export interface NavItem {
  id: string;
  icon: IconName;
  label: string;
  count?: number;
  tone?: "danger";
}

export interface NavSection {
  label?: string;
  items: NavItem[];
}

export interface SideNavProps {
  sections?: NavSection[];
  activeId?: string;
  onSelect?: (id: string) => void;
  context?: ReactNode;
  footer?: ReactNode;
  style?: CSSProperties;
}

export function SideNav({ sections = [], activeId, onSelect, context, footer, style }: SideNavProps) {
  return (
    <aside className="lm-sidenav" style={style}>
      <div className="lm-sidenav__brand">
        <span className="lm-sidenav__wordmark">loom</span>
        <span className="lm-sidenav__wordmark-sub">console</span>
      </div>
      {context ? <div className="lm-sidenav__context">{context}</div> : null}
      <nav className="lm-sidenav__scroll">
        {sections.map((section, si) => (
          <Fragment key={si}>
            {section.label ? <div className="lm-sidenav__section">{section.label}</div> : null}
            {section.items.map((item) => (
              <button
                key={item.id}
                type="button"
                className={"lm-sidenav__item" + (item.id === activeId ? " lm-sidenav__item--active" : "")}
                onClick={() => onSelect && onSelect(item.id)}
                aria-current={item.id === activeId ? "page" : undefined}
              >
                <span className="lm-sidenav__icon"><Icon name={item.icon} size={15} /></span>
                {item.label}
                {item.count != null ? (
                  <span className={"lm-sidenav__count" + (item.tone === "danger" ? " lm-sidenav__count--danger" : "")}>{item.count}</span>
                ) : null}
              </button>
            ))}
          </Fragment>
        ))}
      </nav>
      {footer ? <div className="lm-sidenav__foot">{footer}</div> : null}
    </aside>
  );
}
