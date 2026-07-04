import type { CSSProperties } from "react";
import { Icon } from "./Icon.tsx";

export interface SpinnerProps {
  size?: number;
  label?: string;
  style?: CSSProperties;
}

export function Spinner({ size = 14, label = "Loading", style }: SpinnerProps) {
  return (
    <span className="lm-spinner" role="status" aria-label={label} style={style}>
      <Icon name="loader-circle" size={size} />
    </span>
  );
}
