import type { CSSProperties, ReactNode } from "react";

export function Transcript({ children, style }: { children?: ReactNode; style?: CSSProperties }) {
  return <div className="lm-transcript" style={style}>{children}</div>;
}
