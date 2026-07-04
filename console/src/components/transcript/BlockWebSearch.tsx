import type { CSSProperties } from "react";
import { BlockFrame } from "./blockBase.tsx";
import type { WebSearchResult } from "../../api/types.ts";

export interface BlockWebSearchProps {
  query: string;
  results?: WebSearchResult[];
  style?: CSSProperties;
}

export function BlockWebSearch({ query, results = [], style }: BlockWebSearchProps) {
  return (
    <div style={style}>
      <BlockFrame icon="globe" kind="web search" name={'"' + query + '"'} collapsible defaultOpen meta={<span>{results.length} results</span>}>
        <div className="lm-websearch">
          {results.map((r, i) => (
            <div className="lm-websearch__result" key={i}>
              <span className={"lm-websearch__idx" + (r.cited ? " lm-websearch__idx--cited" : "")}>[{i + 1}]</span>
              <span className="lm-websearch__title">
                {r.title}
                {r.cited ? <span className="lm-websearch__cited-tag">cited</span> : null}
              </span>
              <span className="lm-websearch__url">{r.url}</span>
              {r.snippet ? <span className="lm-websearch__snippet">{r.snippet}</span> : null}
            </div>
          ))}
        </div>
      </BlockFrame>
    </div>
  );
}
