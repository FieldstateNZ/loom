import type { CSSProperties } from "react";
import { Icon, type IconName } from "./Icon.tsx";

export interface IconButtonProps {
  icon: IconName;
  label: string;
  variant?: "ghost" | "secondary" | "danger";
  size?: "sm" | "md";
  disabled?: boolean;
  onClick?: () => void;
  style?: CSSProperties;
}

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
