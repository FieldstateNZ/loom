import type { CSSProperties } from "react";
import { Icon, type IconName } from "./icon.tsx";

/** Props for {@link IconButton}. */
export interface IconButtonProps {
  readonly icon: IconName;
  /** Accessible label, also shown as the native title/tooltip since the button has no visible text. */
  readonly label: string;
  readonly variant?: "ghost" | "secondary" | "danger";
  readonly size?: "sm" | "md";
  readonly disabled?: boolean;
  readonly onClick?: () => void;
  readonly style?: CSSProperties;
}

/** A compact, icon-only button for toolbars and dense UI where a full {@link Button} with text would take up too much space. */
export function IconButton({ icon, label, variant = "ghost", size = "md", disabled, onClick, style }: IconButtonProps) {
  const cls = [
    "lm-iconbtn",
    size === "sm" ? "lm-iconbtn--sm" : "",
    variant !== "ghost" ? "lm-iconbtn--" + variant : "",
  ].filter(Boolean).join(" ");
  return (
    <button type="button" className={cls} aria-label={label} title={label} disabled={disabled} onClick={onClick} style={style}>
      <Icon name={icon} size={size === "sm" ? 14 : 15} />
    </button>
  );
}
