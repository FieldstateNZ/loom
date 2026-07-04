import type { CSSProperties } from "react";
import type { BarItem } from "../../api/types.ts";

/** Props for {@link BarList}. */
export interface BarListProps {
  readonly items?: readonly BarItem[];
  readonly color?: string;
  readonly mono?: boolean;
  readonly onSelect?: (item: BarItem) => void;
  readonly style?: CSSProperties;
}

/** Renders a horizontal list of comparative bars, e.g. for ranked categories or top-N breakdowns. */
export function BarList({ items = [], color = "var(--accent)", mono = false, onSelect, style }: BarListProps) {
  const max = Math.max(...items.map((i) => i.value), 0) || 1;
  return (
    <div className="lm-barlist" style={style}>
      {items.map((item, idx) => {
        const rowCls = "lm-barlist__row" + (onSelect ? " lm-barlist__row--click" : "");
        const bar = <span className="lm-barlist__bar" style={{ width: (item.value / max) * 100 + "%", background: item.color || color }}></span>;
        const lbl = <span className={"lm-barlist__label" + (mono ? " lm-barlist__label--mono" : "")}>{item.label}</span>;
        const val = <span className="lm-barlist__value">{item.display != null ? item.display : item.value}</span>;
        const key = item.key || item.label || idx;
        return onSelect ? (
          <button key={key} type="button" className={rowCls} onClick={() => onSelect(item)}>
            {bar}{lbl}{val}
          </button>
        ) : (
          <div key={key} className={rowCls}>{bar}{lbl}{val}</div>
        );
      })}
    </div>
  );
}
