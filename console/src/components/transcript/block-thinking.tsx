import { useState, type CSSProperties, type ReactNode } from "react";
import { Icon } from "../core/icon.tsx";

export interface BlockThinkingProps {
  duration?: string | undefined;
  defaultOpen?: boolean;
  children?: ReactNode;
  style?: CSSProperties;
}

export function BlockThinking({ duration, defaultOpen = false, children, style }: BlockThinkingProps) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className="lm-thinking" style={style}>
      <button type="button" className="lm-thinking__toggle" onClick={() => setOpen(!open)} aria-expanded={open}>
        <Icon name="brain" size={13} />
        <span>thinking{duration ? " · " + duration : ""}</span>
        <span className={"lm-thinking__chevron" + (open ? " lm-thinking__chevron--open" : "")}><Icon name="chevron-right" size={12} /></span>
      </button>
      {open ? <div className="lm-thinking__body">{children}</div> : null}
    </div>
  );
}
