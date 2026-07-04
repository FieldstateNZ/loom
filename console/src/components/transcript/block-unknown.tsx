import type { CSSProperties } from "react";
import { BlockFrame, JsonPre } from "./block-base.tsx";

export interface BlockUnknownProps {
  type: string;
  data: unknown;
  style?: CSSProperties;
}

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
