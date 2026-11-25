import { AlertTriangle, CheckCircle2, Info, X, XCircle } from "lucide-react";
import type { ReactNode } from "react";

export type NoticeTone = "info" | "success" | "warning" | "error";

interface NoticeProps {
  tone: NoticeTone;
  title?: ReactNode;
  children?: ReactNode;
  /** Buttons for what to do about it. */
  actions?: ReactNode;
  /** Shows a close button when given. */
  onDismiss?: () => void;
  dismissLabel?: string;
}

const icons = {
  info: Info,
  success: CheckCircle2,
  warning: AlertTriangle,
  error: XCircle,
} as const;

/**
 * A message box. Errors are announced at once (`role="alert"`); everything else politely
 * (`role="status"`).
 */
export function Notice({
  tone,
  title,
  children,
  actions,
  onDismiss,
  dismissLabel = "Dismiss",
}: NoticeProps) {
  const Icon = icons[tone];
  return (
    <div className={`notice notice--${tone}`} role={tone === "error" ? "alert" : "status"}>
      <Icon className="notice__icon" size={18} aria-hidden="true" />
      <div className="notice__body">
        {title ? <p className="notice__title">{title}</p> : null}
        {children ? <div className="notice__text">{children}</div> : null}
        {actions ? <div className="button-row notice__actions">{actions}</div> : null}
      </div>
      {onDismiss ? (
        <button
          type="button"
          className="icon-button notice__dismiss"
          onClick={onDismiss}
          aria-label={dismissLabel}
          title={dismissLabel}
        >
          <X size={16} aria-hidden="true" />
        </button>
      ) : null}
    </div>
  );
}

/**
 * A region whose text is announced when it changes. It is always rendered, even when empty,
 * because a live region that appears together with its text is often not announced.
 */
export function LiveMessage({
  message,
  tone = "status",
  className = "field-hint",
}: {
  message: string | null;
  tone?: "status" | "alert";
  className?: string;
}) {
  const toneClass = tone === "alert" ? " field-hint--error" : "";
  return (
    <p
      className={`${className}${toneClass} live-message`}
      role={tone}
      aria-live={tone === "alert" ? "assertive" : "polite"}
    >
      {message ?? ""}
    </p>
  );
}
