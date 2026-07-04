// blockBase — shared chrome for transcript content blocks.
import { useState, type ReactNode } from "react";
import { Icon, type IconName } from "../core/Icon.tsx";

export function JsonPre({ data, label, maxHeight }: { data: unknown; label?: string; maxHeight?: number | string }) {
  const text = typeof data === "string" ? data : JSON.stringify(data, null, 2);
  return (
    <div style={{ minWidth: 0 }}>
      {label ? <p className="lm-pre__label">{label}</p> : null}
      <pre className="lm-pre" style={maxHeight ? { maxHeight } : undefined}>{text}</pre>
    </div>
  );
}

export interface BlockFrameProps {
  icon: IconName;
  kind: string;
  name?: ReactNode;
  meta?: ReactNode;
  tone?: "danger" | undefined;
  collapsible?: boolean;
  defaultOpen?: boolean;
  children?: ReactNode;
}

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
