import type { CSSProperties, KeyboardEvent } from "react";
import { Icon, type IconName } from "../core/Icon.tsx";

export interface InputProps {
  value?: string;
  onChange?: (value: string) => void;
  placeholder?: string;
  type?: string;
  size?: "sm" | "md";
  mono?: boolean;
  invalid?: boolean;
  icon?: IconName;
  disabled?: boolean;
  readOnly?: boolean;
  autoFocus?: boolean;
  onKeyDown?: (e: KeyboardEvent<HTMLInputElement>) => void;
  style?: CSSProperties;
}

export function Input({
  value,
  onChange,
  placeholder,
  type = "text",
  size = "md",
  mono = false,
  invalid = false,
  icon,
  disabled = false,
  readOnly = false,
  autoFocus = false,
  onKeyDown,
  style,
}: InputProps) {
  const cls = [
    "lm-input",
    size === "sm" ? "lm-input--sm" : "",
    mono ? "lm-input--mono" : "",
    invalid ? "lm-input--invalid" : "",
    icon ? "lm-input--with-icon" : "",
  ].filter(Boolean).join(" ");
  const input = (
    <input
      className={cls}
      type={type}
      value={value}
      onChange={(e) => onChange && onChange(e.target.value)}
      placeholder={placeholder}
      disabled={disabled}
      readOnly={readOnly}
      autoFocus={autoFocus}
      onKeyDown={onKeyDown}
      aria-invalid={invalid || undefined}
      style={icon ? undefined : style}
    />
  );
  if (!icon) return input;
  return (
    <span className="lm-input-wrap" style={style}>
      <span className="lm-input-wrap__icon"><Icon name={icon} size={14} /></span>
      {input}
    </span>
  );
}
