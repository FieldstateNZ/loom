import type { CSSProperties } from "react";
import { BlockFrame } from "./block-frame.tsx";
import { JsonPre } from "./json-pre.tsx";

/** Props for {@link BlockCodeExec}. */
export interface BlockCodeExecProps {
  /** Language identifier used for display; defaults to "python". */
  readonly lang?: string | undefined;
  readonly code?: string | undefined;
  readonly stdout?: string | undefined;
  readonly stderr?: string | undefined;
  /** Process exit code; a non-zero value marks the block as failed. */
  readonly exitCode?: number | undefined;
  readonly style?: CSSProperties;
}

/** Collapsible block showing executed code along with its stdout, stderr, and exit code. */
export function BlockCodeExec({ lang = "python", code, stdout, stderr, exitCode, style }: BlockCodeExecProps) {
  const failed = exitCode != null && exitCode !== 0;
  return (
    <div style={style}>
      <BlockFrame
        icon="code"
        kind="code execution"
        name={lang}
        tone={failed ? "danger" : undefined}
        collapsible
        defaultOpen={failed}
        meta={exitCode != null ? (
          <span style={{ color: failed ? "var(--danger)" : "var(--ok)" }}>exit {exitCode}</span>
        ) : null}
      >
        {code ? <JsonPre label="code" data={code} /> : null}
        {stdout ? <JsonPre label="stdout" data={stdout} /> : null}
        {stderr ? <JsonPre label="stderr" data={stderr} /> : null}
      </BlockFrame>
    </div>
  );
}
