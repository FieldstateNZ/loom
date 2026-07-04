import type { CSSProperties } from "react";
import { BlockFrame } from "./block-frame.tsx";
import { JsonPre } from "./json-pre.tsx";
import { Icon } from "../core/icon.tsx";

/** Props for {@link BlockToolUse}. */
export interface BlockToolUseProps {
  readonly name: string;
  /** Label for the mechanism the tool was invoked through, e.g. an MCP server name. */
  readonly via?: string | undefined;
  readonly input?: unknown;
  readonly result?: unknown;
  /** Whether the tool call failed; styles the block and its result as an error. */
  readonly isError?: boolean | undefined;
  /** Overrides the default open/closed state (which otherwise follows isError). */
  readonly defaultOpen?: boolean;
  readonly style?: CSSProperties;
}

/** Collapsible block showing a tool call's name, input, and result (or error). */
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
