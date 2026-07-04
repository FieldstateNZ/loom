import type { CSSProperties, ReactNode } from "react";

export interface FieldProps {
  label?: ReactNode;
  hint?: ReactNode;
  error?: ReactNode;
  required?: boolean;
  children?: ReactNode;
  style?: CSSProperties;
}

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
