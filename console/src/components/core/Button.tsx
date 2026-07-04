import type { CSSProperties, ReactNode } from "react";
import { Icon, type IconName } from "./Icon.tsx";
import { Spinner } from "./Spinner.tsx";

export type ButtonVariant =
  | "primary"
  | "secondary"
  | "ghost"
  | "danger"
  | "danger-secondary";

export interface ButtonProps {
  variant?: ButtonVariant;
  size?: "sm" | "md";
  icon?: IconName;
  iconAfter?: IconName;
  loading?: boolean;
  disabled?: boolean;
  full?: boolean;
  type?: "button" | "submit" | "reset";
  onClick?: () => void;
  children?: ReactNode;
  style?: CSSProperties;
}

export function Button({
  variant = "secondary",
  size = "md",
  icon,
  iconAfter,
  loading = false,
  disabled = false,
  full = false,
  type = "button",
  onClick,
  children,
  style,
}: ButtonProps) {
  const cls = [
    "lm-btn",
    "lm-btn--" + variant,
    size === "sm" ? "lm-btn--sm" : "",
    full ? "lm-btn--full" : "",
  ].filter(Boolean).join(" ");
  const iconSize = size === "sm" ? 13 : 15;
  return (
    <button type={type} className={cls} disabled={disabled || loading} onClick={onClick} style={style}>
      {loading ? <Spinner size={iconSize} /> : icon ? <Icon name={icon} size={iconSize} /> : null}
      {children}
      {iconAfter ? <Icon name={iconAfter} size={iconSize} /> : null}
    </button>
  );
}
