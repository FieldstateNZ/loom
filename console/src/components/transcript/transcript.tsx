import type { CSSProperties, ReactNode } from "react";

/** Vertical container that lays out a conversation's {@link Turn}s in sequence. */
export function Transcript({ children, style }: { readonly children?: ReactNode; readonly style?: CSSProperties }) {
  return <div className="lm-transcript" style={style}>{children}</div>;
}
