import type { CSSProperties, ReactNode } from "react";

export function Kbd({ children, style }: { children?: ReactNode; style?: CSSProperties }) {
  return <kbd className="lm-kbd" style={style}>{children}</kbd>;
}
