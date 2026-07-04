import type { CSSProperties } from "react";
import { BlockFrame, JsonPre } from "./blockBase.tsx";
import { Icon } from "../core/Icon.tsx";

export interface BlockToolUseProps {
  name: string;
  via?: string;
  input?: unknown;
  result?: unknown;
  isError?: boolean;
  defaultOpen?: boolean;
  style?: CSSProperties;
}

export function BlockToolUse({ name, via, input, result, isError = false, defaultOpen, style }: BlockToolUseProps) {
  const open = defaultOpen != null ? defaultOpen : isError;
  return (
    <div style={style}>
      <BlockFrame
        icon="wrench"
        kind="tool use"
        name={name}
        tone={isError ? "danger" : undefined}
        collapsible
        defaultOpen={open}
        meta={
          <>
            {via ? <span>via {via}</span> : null}
            {result !== undefined ? (
              isError
                ? <span style={{ color: "var(--danger)", display: "inline-flex", alignItems: "center", gap: "4px" }}><Icon name="circle-alert" size={12} /> error</span>
                : <span style={{ color: "var(--ok)", display: "inline-flex", alignItems: "center", gap: "4px" }}><Icon name="check" size={12} /> ok</span>
            ) : <span>no result</span>}
          </>
        }
      >
        {input !== undefined ? <JsonPre label="input" data={input} /> : null}
        {result !== undefined ? <JsonPre label={isError ? "error" : "result"} data={result} /> : null}
      </BlockFrame>
    </div>
  );
}
