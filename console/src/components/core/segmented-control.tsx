import type { CSSProperties } from "react";
import { Icon, type IconName } from "./icon.tsx";

/** A single choice within a {@link SegmentedControl}, with an optional display label distinct from its underlying value. */
export interface SegmentOption {
  readonly value: string;
  readonly label?: string;
  readonly icon?: IconName;
  /** Tooltip text shown on hover. */
  readonly title?: string;
}

/** Props for {@link SegmentedControl}. */
export interface SegmentedControlProps {
  /** Plain strings are shorthand for an option whose value and label are the same. */
  readonly options: (string | SegmentOption)[];
  readonly value: string;
  readonly onChange?: (value: string) => void;
  readonly mono?: boolean;
  readonly label?: string;
  readonly style?: CSSProperties;
}

/** A row of mutually exclusive buttons acting like a tab bar, used to switch between a small set of related views or modes. */
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
