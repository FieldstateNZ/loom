import type { CSSProperties, ReactNode } from "react";

/** Props for {@link Switch}. */
export interface SwitchProps {
  readonly checked?: boolean;
  readonly onChange?: (checked: boolean) => void;
  readonly label?: ReactNode;
  readonly disabled?: boolean;
  readonly style?: CSSProperties;
}

/** Toggle switch for boolean settings, optionally paired with a clickable label. */
export function Switch({ checked = false, onChange, label, disabled = false, style }: SwitchProps) {
  const btn = (
    <button
      type="button"
      role="switch"
      className="lm-switch"
      aria-checked={checked}
      aria-label={typeof label === "string" ? label : undefined}
      disabled={disabled}
      onClick={() => onChange && onChange(!checked)}
      style={label ? undefined : style}
    ></button>
  );
  if (!label) return btn;
  return (
    <span className="lm-switch-row" style={style}>
      {btn}
      <span className="lm-switch-row__label" onClick={() => !disabled && onChange && onChange(!checked)}>{label}</span>
    </span>
  );
}
