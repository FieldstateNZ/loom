import type { CSSProperties } from "react";
import { Icon } from "../core/icon.tsx";

/** A selectable option with a distinct display label from its underlying value. */
export interface SelectOption {
  readonly value: string;
  readonly label: string;
}

/** Props for {@link Select}. */
export interface SelectProps {
  /** Plain strings are used as both value and label; use {@link SelectOption} when they differ. */
  readonly options: (string | SelectOption)[];
  readonly value: string;
  readonly onChange?: (value: string) => void;
  readonly size?: "sm" | "md";
  readonly mono?: boolean;
  readonly disabled?: boolean;
  readonly label?: string;
  readonly style?: CSSProperties;
}

/** Styled native `<select>` dropdown for choosing one value from a list of options. */
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
