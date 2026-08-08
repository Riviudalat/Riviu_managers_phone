import { useEffect, useRef } from "react";
import { answerConfirm, useConfirmRequest } from "../confirmStore";

/**
 * Modal confirmation for consequential actions (restore, uninstall, discarding
 * a draft). Replaces `window.confirm`: same blocking semantics for the operator,
 * but themed, and the copy can name the actual consequence.
 */
export function ConfirmHost() {
  const request = useConfirmRequest();
  const confirmRef = useRef<HTMLButtonElement>(null);
  const id = request?.id;

  useEffect(() => {
    if (id === undefined) return;
    confirmRef.current?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        answerConfirm(id, false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [id]);

  if (!request) return null;

  return (
    <div className="confirm-layer">
      <div className="confirm-backdrop" onClick={() => answerConfirm(request.id, false)} />
      <div
        className="confirm-card"
        role="alertdialog"
        aria-modal="true"
        aria-label={request.title}
      >
        <h3>{request.title}</h3>
        {request.message && <p>{request.message}</p>}
        <div className="confirm-actions">
          <button type="button" onClick={() => answerConfirm(request.id, false)}>
            {request.cancelLabel ?? "Hủy"}
          </button>
          <button
            ref={confirmRef}
            type="button"
            className={request.danger ? "danger" : "primary"}
            onClick={() => answerConfirm(request.id, true)}
          >
            {request.confirmLabel ?? "Tiếp tục"}
          </button>
        </div>
      </div>
    </div>
  );
}
