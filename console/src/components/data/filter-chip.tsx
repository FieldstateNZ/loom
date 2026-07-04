import type { CSSProperties } from "react";
import { Icon } from "../core/icon.tsx";

/** Props for {@link FilterChip}. */
export interface FilterChipProps {
  readonly field: string;
  readonly op?: string;
  readonly value: string;
  readonly onRemove?: () => void;
  readonly style?: CSSProperties;
}

/** Renders an active filter as a removable chip, e.g. "field = value" in a filter bar. */
export function FilterChip({ field, op = "=", value, onRemove, style }: FilterChipProps) {
  return (
    <span className="lm-fchip" style={style}>
      <span className="lm-fchip__dim">{field}</span>
      <span className="lm-fchip__dim">{op}</span>
      <span>{value}</span>
      {onRemove ? (
        <button type="button" className="lm-fchip__x" aria-label={"Remove filter " + field} onClick={onRemove}>
          <Icon name="x" size={12} />
        </button>
      ) : null}
    </span>
  );
}
