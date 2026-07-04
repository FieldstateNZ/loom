import type { CSSProperties, ReactNode } from "react";

/** Renders a plain text content block, splitting on blank lines into separate paragraphs. */
export function BlockText({ children, style }: { readonly children?: ReactNode; readonly style?: CSSProperties }) {
  const content = typeof children === "string"
    ? children.split(/\n\n+/).map((p, i) => <p key={i}>{p}</p>)
    : children;
  return <div className="lm-blocktext" style={style}>{content}</div>;
}
