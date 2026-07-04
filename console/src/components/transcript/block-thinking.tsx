import { useState, type CSSProperties, type ReactNode } from "react";
import { Icon } from "../core/icon.tsx";

/** Props for {@link BlockThinking}. */
export interface BlockThinkingProps {
  /** How long the model spent thinking, shown next to the toggle label. */
  readonly duration?: string | undefined;
  /** Whether the thinking content starts expanded; defaults to collapsed. */
  readonly defaultOpen?: boolean;
  readonly children?: ReactNode;
  readonly style?: CSSProperties;
}

/** Collapsible block that reveals a model's extended-thinking content behind a toggle. */
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
