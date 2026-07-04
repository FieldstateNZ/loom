import { useState, type CSSProperties } from "react";
import { Icon } from "../core/icon.tsx";
import { Button } from "../core/button.tsx";

/** Props for {@link RevealOnce}. */
export interface RevealOnceProps {
  /** The secret value to display and let the user copy; the caller should not persist it after this render. */
  readonly secret: string;
  readonly heading?: string;
  /** Overrides the default warning message shown below the secret. */
  readonly warning?: string;
  /** Called after the user copies the secret to the clipboard. */
  readonly onCopy?: () => void;
  readonly style?: CSSProperties;
}

/** Displays a one-time secret (e.g. API key) with a copy button, since it cannot be retrieved again after this view. */
export function RevealOnce({ secret, heading = "Copy your key now", warning, onCopy, style }: RevealOnceProps) {
  const [copied, setCopied] = useState(false);
  const copy = () => {
    try {
      if (navigator.clipboard) navigator.clipboard.writeText(secret);
    } catch { /* selection fallback: the field is user-select: all */ }
    setCopied(true);
    if (onCopy) onCopy();
    setTimeout(() => setCopied(false), 2500);
  };
  return (
    <div className="lm-reveal" style={style}>
      <div className="lm-reveal__head">
        <Icon name="key" size={15} color="var(--warn)" />
        {heading}
      </div>
      <div className="lm-reveal__secret-row">
        <code className="lm-reveal__secret">{secret}</code>
        <Button variant={copied ? "secondary" : "primary"} icon={copied ? "check" : "copy"} onClick={copy} style={{ alignSelf: "center" }}>
          {copied ? "Copied" : "Copy"}
        </Button>
      </div>
      <div className="lm-reveal__warning">
        <Icon name="triangle-alert" size={14} color="var(--warn)" style={{ marginTop: "1px" }} />
        <span>
          <strong>This is the only time Loom will show this key.</strong>{" "}
          {warning || "Store it in your secret manager before closing — it cannot be retrieved again, only revoked and reissued."}
        </span>
      </div>
    </div>
  );
}
