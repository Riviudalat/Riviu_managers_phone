import { dismissToast, useToasts, type ToastKind } from "../toastStore";
import { IconCheck, IconClose, IconInfo, IconWarning } from "./Icons";

function ToastIcon({ kind }: { kind: ToastKind }) {
  if (kind === "ok") return <IconCheck size={18} />;
  if (kind === "warn" || kind === "error") return <IconWarning size={18} />;
  return <IconInfo size={18} />;
}

/**
 * Notification stack. Replaces `window.alert` so results and failures read in
 * the app's own voice, stay dismissible, and never block the operator mid-task.
 */
export function ToastHost() {
  const toasts = useToasts();
  if (toasts.length === 0) return null;

  return (
    <div className="toast-host" role="region" aria-label="Thông báo">
      {toasts.map((toast) => (
        <div
          key={toast.id}
          className={`toast toast-${toast.kind}`}
          role={toast.kind === "error" ? "alert" : "status"}
          aria-live={toast.kind === "error" ? "assertive" : "polite"}
        >
          <span className="toast-icon">
            <ToastIcon kind={toast.kind} />
          </span>
          <div className="toast-copy">
            <strong>{toast.title}</strong>
            {toast.detail && <p>{toast.detail}</p>}
          </div>
          <button
            type="button"
            className="toast-close"
            title="Đóng"
            aria-label={`Đóng thông báo: ${toast.title}`}
            onClick={() => dismissToast(toast.id)}
          >
            <IconClose size={13} />
          </button>
        </div>
      ))}
    </div>
  );
}
