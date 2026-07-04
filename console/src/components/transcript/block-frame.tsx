// Shared chrome for transcript content blocks.
import { useState, type ReactNode } from "react";
import { Icon, type IconName } from "../core/icon.tsx";

/** Props for {@link BlockFrame}. */
export interface BlockFrameProps {
  readonly icon: IconName;
  readonly kind: string;
  readonly name?: ReactNode;
  readonly meta?: ReactNode;
  readonly tone?: "danger" | undefined;
  readonly collapsible?: boolean;
  readonly defaultOpen?: boolean;
  readonly children?: ReactNode;
}

/**
 * The consistent header (icon + kind + name + meta) every transcript block sits
 * inside, optionally collapsible. Keeping the chrome here means each block type
 * only supplies its own body, so they all look and behave the same.
 */
export function BlockFrame({ icon, kind, name, meta, tone, collapsible = false, defaultOpen = true, children }: BlockFrameProps) {
  const [open, setOpen] = useState(defaultOpen);
  const headProps = {
    className: "lm-block__head",
    type: collapsible ? ("button" as const) : undefined,
    onClick: collapsible ? () => setOpen(!open) : undefined,
    "aria-expanded": collapsible ? open : undefined,
  };
  const headContent = (
    <>
      <span className="lm-block__icon"><Icon name={icon} size={13} /></span>
      <span className="lm-block__kind">{kind}</span>
      {name ? <span className="lm-block__name">{name}</span> : null}
      <span className="lm-block__meta">
        {meta}
        {collapsible ? <span className={"lm-block__chevron" + (open ? " lm-block__chevron--open" : "")}><Icon name="chevron-right" size={13} /></span> : null}
      </span>
    </>
  );
  return (
    <div className={"lm-block" + (tone === "danger" ? " lm-block--danger" : "")}>
      {collapsible ? (
        <button {...headProps}>{headContent}</button>
      ) : (
        <div {...headProps}>{headContent}</div>
      )}
      {(!collapsible || open) && children ? <div className="lm-block__body">{children}</div> : null}
    </div>
  );
}
