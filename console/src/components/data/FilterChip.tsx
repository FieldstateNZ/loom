import type { CSSProperties } from "react";
import { Icon } from "../core/Icon.tsx";

export interface FilterChipProps {
  field: string;
  op?: string;
  value: string;
  onRemove?: () => void;
  style?: CSSProperties;
}

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
