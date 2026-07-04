import type { CSSProperties, ReactNode } from "react";
import { Icon, type IconName } from "./icon.tsx";
import { Spinner } from "./spinner.tsx";

/** The visual style a {@link Button} can be rendered in, from the primary call-to-action down to destructive actions. */
export type ButtonVariant =
  | "primary"
  | "secondary"
  | "ghost"
  | "danger"
  | "danger-secondary";

/** Props for {@link Button}. */
export interface ButtonProps {
  readonly variant?: ButtonVariant;
  readonly size?: "sm" | "md";
  readonly icon?: IconName;
  readonly iconAfter?: IconName;
  readonly loading?: boolean;
  readonly disabled?: boolean;
  readonly full?: boolean;
  readonly type?: "button" | "submit" | "reset";
  readonly onClick?: () => void;
  readonly children?: ReactNode;
  readonly style?: CSSProperties;
}

/** The standard clickable button used throughout the console, supporting icons, loading state, and multiple visual variants. */
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
