import { useState, type CSSProperties } from "react";
import { Icon } from "../core/icon.tsx";
import { Button } from "../core/button.tsx";
import { Input } from "./input.tsx";

export interface SecretInputProps {
  isSet?: boolean;
  meta?: string | null;
  placeholder?: string;
  onSave?: (value: string) => void;
  saveLabel?: string;
  note?: string;
  style?: CSSProperties;
}

export function SecretInput({
  isSet = false,
  meta,
  placeholder = "Paste secret…",
  onSave,
  saveLabel = "Save",
  note = "Write-only. Loom stores this encrypted and never displays it again.",
  style,
}: SecretInputProps) {
  const [editing, setEditing] = useState(!isSet);
  const [value, setValue] = useState("");

  if (!editing) {
    return (
      <div className="lm-secret" style={style}>
        <div className="lm-secret__set">
          <Icon name="shield" size={14} color="var(--ok)" />
          <span className="lm-secret__mask">••••••••••••</span>
          {meta ? <span className="lm-secret__meta">{meta}</span> : <span className="lm-secret__meta" />}
          <Button size="sm" icon="rotate-cw" onClick={() => { setValue(""); setEditing(true); }}>Rotate</Button>
        </div>
      </div>
    );
  }
  return (
    <div className="lm-secret" style={style}>
      <div className="lm-secret__row">
        <Input type="password" mono value={value} onChange={setValue} placeholder={placeholder} style={{ flex: 1 }} autoFocus={isSet} />
        <Button variant="primary" disabled={!value} onClick={() => { onSave && onSave(value); setValue(""); setEditing(false); }}>{saveLabel}</Button>
        {isSet ? <Button variant="ghost" onClick={() => setEditing(false)}>Cancel</Button> : null}
      </div>
      <div className="lm-secret__note">
        <Icon name="eye-off" size={13} />
        {note}
      </div>
    </div>
  );
}
