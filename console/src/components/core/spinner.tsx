import type { CSSProperties } from "react";
import { Icon } from "./icon.tsx";

/** Props for {@link Spinner}. */
export interface SpinnerProps {
  readonly size?: number;
  readonly label?: string;
  readonly style?: CSSProperties;
}

/** An animated loading indicator; used wherever content or an action is pending. */
export function Spinner({ size = 14, label = "Loading", style }: SpinnerProps) {
  return (
    <span className="lm-spinner" role="status" aria-label={label} style={style}>
      <Icon name="loader-circle" size={size} />
    </span>
  );
}
