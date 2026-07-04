import { useState, type CSSProperties } from "react";
import { Icon } from "../core/Icon.tsx";
import { Button } from "../core/Button.tsx";

export interface RevealOnceProps {
  secret: string;
  heading?: string;
  warning?: string;
  onCopy?: () => void;
  style?: CSSProperties;
}

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
