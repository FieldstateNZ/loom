import type { CSSProperties } from "react";
import { Icon } from "../core/icon.tsx";
import { formatTokens } from "../../lib/format.ts";

/** Props for {@link CacheMarker}. */
export interface CacheMarkerProps {
  /** Whether this marks a cache read or a cache write; defaults to "read". */
  readonly kind?: "read" | "write";
  /** Number of tokens served/written from/to cache, shown alongside the label. */
  readonly tokens?: number;
  readonly style?: CSSProperties;
}

/** Inline divider marking where a cache read or write occurred within a turn's usage. */
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
