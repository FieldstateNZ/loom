import type { CSSProperties } from "react";
import { Icon, type IconName } from "./Icon.tsx";

export interface SegmentOption {
  value: string;
  label?: string;
  icon?: IconName;
  title?: string;
}

export interface SegmentedControlProps {
  options: (string | SegmentOption)[];
  value: string;
  onChange?: (value: string) => void;
  mono?: boolean;
  label?: string;
  style?: CSSProperties;
}

export function SegmentedControl({ options, value, onChange, mono = false, label, style }: SegmentedControlProps) {
  return (
    <div className={"lm-seg" + (mono ? " lm-seg--mono" : "")} role="group" aria-label={label} style={style}>
      {options.map((opt) => {
        const o: SegmentOption = typeof opt === "string" ? { value: opt, label: opt } : opt;
        return (
          <button
            key={o.value}
            type="button"
            className="lm-seg__btn"
            aria-pressed={o.value === value}
            onClick={() => onChange && onChange(o.value)}
            title={o.title}
          >
            {o.icon ? <Icon name={o.icon} size={13} /> : null}
            {o.label}
          </button>
        );
      })}
    </div>
  );
}
