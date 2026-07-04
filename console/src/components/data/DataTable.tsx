import type { CSSProperties, ReactNode } from "react";

export interface Column<T> {
  key: string;
  label: ReactNode;
  width?: string;
  align?: "left" | "right" | "center";
  mono?: boolean;
  muted?: boolean;
  render?: (row: T) => ReactNode;
}

export interface DataTableProps<T> {
  columns: Column<T>[];
  rows: T[];
  rowKey?: keyof T | ((row: T) => string | number);
  onRowClick?: (row: T) => void;
  dense?: boolean;
  empty?: ReactNode;
  style?: CSSProperties;
}

export function DataTable<T>({
  columns,
  rows,
  rowKey,
  onRowClick,
  dense = false,
  empty,
  style,
}: DataTableProps<T>) {
  const cellCls = (col: { align?: string; mono?: boolean; muted?: boolean }) =>
    [
      col.align === "right" ? "lm-table__right" : col.align === "center" ? "lm-table__center" : "",
      col.mono ? "lm-table__mono" : "",
      col.muted ? "lm-table__muted" : "",
    ].filter(Boolean).join(" ") || undefined;
  const keyOf = (row: T, i: number): string | number =>
    typeof rowKey === "function" ? rowKey(row) : rowKey ? (row[rowKey] as string | number) : i;
  return (
    <div className="lm-table-scroll" style={style}>
      <table className={["lm-table", dense ? "lm-table--dense" : "", onRowClick ? "lm-table--click" : ""].filter(Boolean).join(" ")}>
        <thead>
          <tr>
            {columns.map((col) => (
              <th key={col.key} className={cellCls({ align: col.align })} style={col.width ? { width: col.width } : undefined}>
                {col.label}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.length === 0 && empty ? (
            <tr>
              <td colSpan={columns.length} style={{ height: "auto", padding: 0, borderBottom: 0 }}>{empty}</td>
            </tr>
          ) : (
            rows.map((row, i) => (
              <tr key={keyOf(row, i)} onClick={onRowClick ? () => onRowClick(row) : undefined}>
                {columns.map((col) => (
                  <td key={col.key} className={cellCls(col)}>
                    {col.render ? col.render(row) : ((row as Record<string, unknown>)[col.key] as ReactNode)}
                  </td>
                ))}
              </tr>
            ))
          )}
        </tbody>
      </table>
    </div>
  );
}
