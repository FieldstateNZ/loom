import { useEffect, type ReactNode } from "react";
import { Icon, type IconName } from "../core/icon.tsx";
import { IconButton } from "../core/icon-button.tsx";

/** Props for {@link Dialog}. */
export interface DialogProps {
  /** Whether the dialog is currently shown; when false the component renders nothing. */
  readonly open: boolean;
  /** Called when the dialog should close (Escape key, backdrop click, or close button). */
  readonly onClose?: () => void;
  readonly title?: ReactNode;
  readonly icon?: IconName;
  /** Styles the dialog as a destructive/danger confirmation. */
  readonly danger?: boolean;
  /** Max width of the dialog in pixels; defaults to 440. */
  readonly width?: number;
  readonly footer?: ReactNode;
  readonly children?: ReactNode;
}

/** Modal overlay dialog with an optional icon, title, footer, and Escape-to-close behavior. */
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
