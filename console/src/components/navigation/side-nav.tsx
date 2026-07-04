import { Fragment, type CSSProperties, type ReactNode } from "react";
import { Icon, type IconName } from "../core/icon.tsx";

/** A single selectable entry in a {@link SideNav} section. */
export interface NavItem {
  readonly id: string;
  readonly icon: IconName;
  readonly label: string;
  /** Optional badge count shown next to the label. */
  readonly count?: number;
  /** When "danger", the count badge is styled to draw attention. */
  readonly tone?: "danger";
}

/** A labeled group of {@link NavItem}s rendered together in a {@link SideNav}. */
export interface NavSection {
  readonly label?: string;
  readonly items: NavItem[];
}

/** Props for {@link SideNav}. */
export interface SideNavProps {
  readonly sections?: NavSection[];
  /** id of the currently active/selected nav item. */
  readonly activeId?: string;
  /** Called with the item's id when a nav item is clicked. */
  readonly onSelect?: (id: string) => void;
  /** Content rendered above the nav sections, e.g. an org/workspace switcher. */
  readonly context?: ReactNode;
  readonly footer?: ReactNode;
  readonly style?: CSSProperties;
}

/** Vertical navigation sidebar showing grouped sections of selectable items. */
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
