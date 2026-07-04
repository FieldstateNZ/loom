import type { CSSProperties } from "react";
import { Icon } from "../core/icon.tsx";

export interface SelectOption {
  value: string;
  label: string;
}

export interface SelectProps {
  options: (string | SelectOption)[];
  value: string;
  onChange?: (value: string) => void;
  size?: "sm" | "md";
  mono?: boolean;
  disabled?: boolean;
  label?: string;
  style?: CSSProperties;
}

export function Select({ options, value, onChange, size = "md", mono = false, disabled = false, label, style }: SelectProps) {
  const cls = [
    "lm-select",
    size === "sm" ? "lm-select--sm" : "",
    mono ? "lm-select--mono" : "",
  ].filter(Boolean).join(" ");
  return (
    <span className="lm-select-wrap" style={style}>
      <select
        className={cls}
        value={value}
        onChange={(e) => onChange && onChange(e.target.value)}
        disabled={disabled}
        aria-label={label}
      >
        {options.map((opt) => {
          const o: SelectOption = typeof opt === "string" ? { value: opt, label: opt } : opt;
          return <option key={o.value} value={o.value}>{o.label}</option>;
        })}
      </select>
      <span className="lm-select-wrap__chevron"><Icon name="chevron-down" size={14} /></span>
    </span>
  );
}
