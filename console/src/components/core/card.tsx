import type { CSSProperties, ReactNode } from "react";

/** Props for {@link Card}. */
export interface CardProps {
  readonly eyebrow?: ReactNode;
  readonly title?: ReactNode;
  readonly actions?: ReactNode;
  readonly footer?: ReactNode;
  /** When true, removes the body's default padding so content can sit flush against the card edges. */
  readonly flush?: boolean;
  readonly children?: ReactNode;
  readonly style?: CSSProperties;
}

/** A bordered content container with an optional header (eyebrow/title/actions) and footer, used to group related content. */
export function Card({ eyebrow, title, actions, footer, flush = false, children, style }: CardProps) {
  const hasHead = eyebrow || title || actions;
  return (
    <section className="lm-card" style={style}>
      {hasHead ? (
        <header className="lm-card__head">
          <div style={{ minWidth: 0 }}>
            {eyebrow ? <div className="lm-card__eyebrow">{eyebrow}</div> : null}
            {title ? <h3 className="lm-card__title">{title}</h3> : null}
          </div>
          {actions ? <div style={{ display: "flex", gap: "6px", alignItems: "center", flexShrink: 0 }}>{actions}</div> : null}
        </header>
      ) : null}
      <div className={"lm-card__body" + (flush ? " lm-card__body--flush" : "")}>{children}</div>
      {footer ? <footer className="lm-card__foot">{footer}</footer> : null}
    </section>
  );
}
