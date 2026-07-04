import type { CSSProperties, ReactNode } from "react";

/** Props for {@link Field}. */
export interface FieldProps {
  readonly label?: ReactNode;
  readonly hint?: ReactNode;
  readonly error?: ReactNode;
  readonly required?: boolean;
  readonly children?: ReactNode;
  readonly style?: CSSProperties;
}

/** Wraps a form control with a label, required marker, and optional hint or error text. */
export function Field({ label, hint, error, required, children, style }: FieldProps) {
  return (
    <div className="lm-field" style={style}>
      {label ? (
        <label className="lm-field__label">
          {label}
          {required ? <span className="lm-field__req" title="Required">*</span> : null}
        </label>
      ) : null}
      {children}
      {error ? <p className="lm-field__error">{error}</p> : hint ? <p className="lm-field__hint">{hint}</p> : null}
    </div>
  );
}
