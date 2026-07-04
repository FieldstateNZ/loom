import type { CSSProperties, ReactNode } from "react";

export function BlockText({ children, style }: { children?: ReactNode; style?: CSSProperties }) {
  const content = typeof children === "string"
    ? children.split(/\n\n+/).map((p, i) => <p key={i}>{p}</p>)
    : children;
  return <div className="lm-blocktext" style={style}>{content}</div>;
}
