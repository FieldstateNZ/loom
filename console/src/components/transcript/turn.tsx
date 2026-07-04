import type { CSSProperties, ReactNode } from "react";
import { formatMoney, formatMs, formatTokens } from "../../lib/format.ts";
import type { TurnUsage } from "../../api/types.ts";

/** Props for {@link Turn}. */
export interface TurnProps {
  /** Who produced this turn; defaults to "assistant". */
  readonly role?: "user" | "assistant" | "system";
  readonly time?: string | undefined;
  readonly model?: string | undefined;
  /** Token/cost/latency usage summary shown in the turn header, if available. */
  readonly usage?: TurnUsage | undefined;
  readonly children?: ReactNode;
  readonly style?: CSSProperties;
}

/** Renders a single turn of a conversation (role, timing/usage metadata, and its content blocks). */
export function Turn({ role = "assistant", time, model, usage, children, style }: TurnProps) {
  return (
    <article className="lm-turn" style={style}>
      <div style={{ display: "flex", justifyContent: "center" }}>
        <span className={"lm-turn__node" + (role !== "user" ? " lm-turn__node--" + role : "")}></span>
      </div>
      <div className="lm-turn__head">
        <span className="lm-turn__role">{role}</span>
        {time ? <span className="lm-turn__head-dim">{time}</span> : null}
        {model ? <span className="lm-turn__head-dim">{model}</span> : null}
      </div>
      {usage ? (
        <div className="lm-turn__usage" aria-label="Turn usage">
          {usage.cost != null ? <span className="lm-turn__usage-cost">{formatMoney(usage.cost)}</span> : null}
          {usage.inTok != null ? <span>{formatTokens(usage.inTok)} in · {formatTokens(usage.outTok || 0)} out</span> : null}
          {usage.cacheRead ? <span className="lm-turn__usage-cache--read">cache r {formatTokens(usage.cacheRead)}</span> : null}
          {usage.cacheWrite ? <span className="lm-turn__usage-cache--write">cache w {formatTokens(usage.cacheWrite)}</span> : null}
          {usage.ms != null ? <span>{formatMs(usage.ms)}</span> : null}
        </div>
      ) : null}
      <div className="lm-turn__blocks">{children}</div>
    </article>
  );
}
