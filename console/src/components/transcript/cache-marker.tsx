import type { CSSProperties } from "react";
import { Icon } from "../core/icon.tsx";
import { formatTokens } from "../../lib/format.ts";

export interface CacheMarkerProps {
  kind?: "read" | "write";
  tokens?: number;
  style?: CSSProperties;
}

export function CacheMarker({ kind = "read", tokens, style }: CacheMarkerProps) {
  const title = kind === "read"
    ? "Prompt prefix served from cache — billed at the reduced cache-read rate."
    : "Prompt prefix written to cache — billed once at the cache-write rate, cheaper on every reuse.";
  return (
    <div className={"lm-cachemark lm-cachemark--" + kind} style={style} title={title}>
      <span className="lm-cachemark__rule"></span>
      <span className="lm-cachemark__label">
        <Icon name="database" size={11} />
        cache {kind}{tokens != null ? " · " + formatTokens(tokens) + " tok" : ""}
      </span>
      <span className="lm-cachemark__rule"></span>
    </div>
  );
}
