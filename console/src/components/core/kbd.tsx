import type { CSSProperties, ReactNode } from "react";

/** Renders text styled as a keyboard key (e.g. for displaying a shortcut like `Ctrl+K`). */
export function Kbd({ children, style }: { readonly children?: ReactNode; readonly style?: CSSProperties }) {
  return <kbd className="lm-kbd" style={style}>{children}</kbd>;
}
