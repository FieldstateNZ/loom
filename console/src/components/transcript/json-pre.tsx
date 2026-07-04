/** Props for {@link JsonPre}. */
export interface JsonPreProps {
  /** The value to render; strings are shown verbatim, everything else pretty-printed. */
  readonly data: unknown;
  readonly label?: string;
  readonly maxHeight?: number | string;
}

/**
 * Renders a tool/result payload as pretty-printed JSON. Used throughout the
 * transcript so any provider value can be shown faithfully without special-casing.
 */
export function JsonPre({ data, label, maxHeight }: JsonPreProps) {
  const text = typeof data === "string" ? data : JSON.stringify(data, null, 2);
  return (
    <div style={{ minWidth: 0 }}>
      {label ? <p className="lm-pre__label">{label}</p> : null}
      <pre className="lm-pre" style={maxHeight ? { maxHeight } : undefined}>{text}</pre>
    </div>
  );
}
