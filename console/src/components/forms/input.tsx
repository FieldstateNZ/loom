import type { CSSProperties, KeyboardEvent } from "react";
import { Icon, type IconName } from "../core/icon.tsx";

/** Props for {@link Input}. */
export interface InputProps {
  readonly value?: string;
  readonly onChange?: (value: string) => void;
  readonly placeholder?: string;
  readonly type?: string;
  readonly size?: "sm" | "md";
  readonly mono?: boolean;
  readonly invalid?: boolean;
  readonly icon?: IconName;
  readonly disabled?: boolean;
  readonly readOnly?: boolean;
  readonly autoFocus?: boolean;
  readonly onKeyDown?: (e: KeyboardEvent<HTMLInputElement>) => void;
  readonly style?: CSSProperties;
}

/** Styled text input, optionally with a leading icon, used throughout forms for single-line values. */
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
