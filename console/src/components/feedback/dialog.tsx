import { useEffect, type ReactNode } from "react";
import { Icon, type IconName } from "../core/icon.tsx";
import { IconButton } from "../core/icon-button.tsx";

export interface DialogProps {
  open: boolean;
  onClose?: () => void;
  title?: ReactNode;
  icon?: IconName;
  danger?: boolean;
  width?: number;
  footer?: ReactNode;
  children?: ReactNode;
}

export function Dialog({ open, onClose, title, icon, danger = false, width = 440, footer, children }: DialogProps) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape" && onClose) onClose(); };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onClose]);
  if (!open) return null;
  const iconName: IconName | null = icon || (danger ? "triangle-alert" : null);
  return (
    <div className="lm-dialog-overlay" onMouseDown={(e) => { if (e.target === e.currentTarget && onClose) onClose(); }}>
      <div className="lm-dialog" role="dialog" aria-modal="true" aria-label={typeof title === "string" ? title : undefined} style={{ maxWidth: width }}>
        <div className="lm-dialog__head">
          {iconName ? <Icon name={iconName} size={17} color={danger ? "var(--danger)" : "var(--fg-3)"} /> : null}
          <h2 className="lm-dialog__title">{title}</h2>
          {onClose ? <IconButton icon="x" label="Close" onClick={onClose} /> : null}
        </div>
        <div className="lm-dialog__body">{children}</div>
        {footer ? <div className="lm-dialog__foot">{footer}</div> : null}
      </div>
    </div>
  );
}
