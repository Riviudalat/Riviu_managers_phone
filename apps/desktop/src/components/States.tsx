import type { ReactNode } from "react";
import { IconCheck, IconInfo, IconWarning } from "./Icons";

/**
 * Nothing-here state. Every list and page reports emptiness the same way:
 * what is missing, and the one action that fills it. `compact` is the inline
 * variant for lists inside a panel; the default is the full-page card.
 */
export function EmptyState({
  icon,
  title,
  hint,
  action,
  compact = false,
}: {
  icon?: ReactNode;
  title: string;
  /** How to get out of the empty state. Skip only when it is truly obvious. */
  hint?: ReactNode;
  action?: ReactNode;
  compact?: boolean;
}) {
  return (
    <div className={`empty-state ${compact ? "compact" : ""}`}>
      {icon && <span className="empty-state-icon">{icon}</span>}
      <div className="empty-state-copy">
        <strong>{title}</strong>
        {hint && <p>{hint}</p>}
      </div>
      {action && <div className="empty-state-action">{action}</div>}
    </div>
  );
}

/**
 * Inline notice pinned to the surface it describes — for conditions the
 * operator must keep seeing, unlike a toast that reports a finished action.
 */
export type NoticeTone = "info" | "success" | "warning" | "error";

export function StatusNotice({
  tone,
  children,
  action,
}: {
  tone: NoticeTone;
  children: ReactNode;
  action?: ReactNode;
}) {
  const urgent = tone === "warning" || tone === "error";
  return (
    <div
      className={`banner banner-${tone}`}
      role={urgent ? "alert" : "status"}
      aria-live={urgent ? "assertive" : "polite"}
    >
      <span className="banner-icon" aria-hidden="true">
        {tone === "info" ? (
          <IconInfo size={16} />
        ) : tone === "success" ? (
          <IconCheck size={16} />
        ) : (
          <IconWarning size={16} />
        )}
      </span>
      <div className="banner-copy">{children}</div>
      {action && <div className="banner-action">{action}</div>}
    </div>
  );
}

/** Compatibility wrapper while existing call sites migrate to the typed notice names. */
export function Banner({
  tone = "warn",
  children,
  action,
}: {
  tone?: "info" | "warn" | "error";
  children: ReactNode;
  action?: ReactNode;
}) {
  return (
    <StatusNotice tone={tone === "warn" ? "warning" : tone} action={action}>
      {children}
    </StatusNotice>
  );
}

/** Waiting state. Announced politely so screen readers report the wait once. */
export function LoadingState({ label = "Đang tải…" }: { label?: string }) {
  return (
    <div className="loading-state" role="status" aria-live="polite">
      <span className="loading-spinner" aria-hidden />
      {label}
    </div>
  );
}
