import type { CSSProperties } from "react";
import { BlockFrame } from "./block-frame.tsx";
import { JsonPre } from "./json-pre.tsx";

/** Props for {@link BlockUnknown}. */
export interface BlockUnknownProps {
  /** The unrecognized block's provider-reported type name. */
  readonly type: string;
  readonly data: unknown;
  readonly style?: CSSProperties;
}

/** Fallback block for provider content types Loom doesn't recognize yet; renders the raw data as JSON. */
export function BlockUnknown({ type, data, style }: BlockUnknownProps) {
  return (
    <div style={style}>
      <BlockFrame
        icon="scroll-text"
        kind="unknown block"
        name={type}
        collapsible
        defaultOpen={false}
        meta={<span title="Forward-compatible: unrecognized provider blocks render as raw JSON and never break the transcript.">raw json</span>}
      >
        <JsonPre data={data} />
      </BlockFrame>
    </div>
  );
}
