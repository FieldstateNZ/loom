import type { CSSProperties } from "react";
import { Icon } from "../core/icon.tsx";
import { formatMoney } from "../../lib/format.ts";

/** Props for {@link BudgetBar}. */
export interface BudgetBarProps {
  readonly spent?: number;
  readonly cap?: number | null;
  readonly window?: string | null;
  readonly mode?: "block" | "warn";
  readonly labels?: boolean;
  readonly style?: CSSProperties;
}

/** Shows spend against a budget cap as a filled track, warning or blocking once the cap is reached depending on `mode`. */
export function BudgetBar({ spent = 0, cap, window: win, mode = "block", labels = false, style }: BudgetBarProps) {
  const ratio = cap ? spent / cap : 0;
  const pct = Math.min(ratio, 1) * 100;
  const over = ratio >= 1;
  const color = over
    ? (mode === "block" ? "var(--danger)" : "var(--warn)")
    : ratio >= 0.75 ? "var(--warn)" : "var(--accent)";
  const blocked = over && mode === "block";
  if (!cap) {
    return (
      <div className="lm-budget" style={style}>
        {labels ? (
          <div className="lm-budget__labels">
            <span className="lm-budget__spent">{formatMoney(spent)}</span>
            <span className="lm-budget__cap">no cap</span>
          </div>
        ) : null}
        <div className="lm-budget__track" role="presentation"></div>
      </div>
    );
  }
  return (
    <div className="lm-budget" style={style}>
      {labels ? (
        <div className="lm-budget__labels">
          <span className="lm-budget__spent">{formatMoney(spent)}</span>
          {blocked ? (
            <span className="lm-budget__blocked"><Icon name="ban" size={11} /> Blocked</span>
          ) : (
            <span className="lm-budget__cap">of {formatMoney(cap)}{win ? " · " + win : ""}</span>
          )}
        </div>
      ) : null}
      <div
        className="lm-budget__track"
        role="meter"
        aria-valuemin={0}
        aria-valuemax={cap}
        aria-valuenow={Math.min(spent, cap)}
        aria-label={"Budget: " + formatMoney(spent) + " of " + formatMoney(cap) + (win ? " " + win : "")}
        title={formatMoney(spent) + " of " + formatMoney(cap) + (win ? " · " + win : "") + " (" + Math.round(ratio * 100) + "%)"}
      >
        <div className="lm-budget__fill" style={{ width: pct + "%", background: color }}></div>
      </div>
    </div>
  );
}
