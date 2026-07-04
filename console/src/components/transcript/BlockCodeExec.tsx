import type { CSSProperties } from "react";
import { BlockFrame, JsonPre } from "./blockBase.tsx";

export interface BlockCodeExecProps {
  lang?: string;
  code?: string;
  stdout?: string;
  stderr?: string;
  exitCode?: number;
  style?: CSSProperties;
}

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
