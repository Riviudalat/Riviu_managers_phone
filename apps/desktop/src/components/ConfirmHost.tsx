import { useEffect, useRef, useState } from "react";
import { answerConfirm, useConfirmRequest } from "../confirmStore";

/**
 * Modal confirmation for consequential actions (restore, uninstall, discarding
 * a draft). Replaces `window.confirm`: same blocking semantics for the operator,
 * but themed, and the copy can name the actual consequence.
 *
 * It also answers the *prompt* requests on the same queue — renaming a phone, numbering it —
 * because a second modal host would be a second layer that can open on top of a dialog
 * somebody is already answering. A prompt is a confirm with a field in it and Enter wired to
 * the confirm button; everything else about the dialog is identical.
 */
export function ConfirmHost() {
  const request = useConfirmRequest();
  const confirmRef = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const cardRef = useRef<HTMLDivElement>(null);
  const [text, setText] = useState("");
  const id = request?.id;
  const prompt = request?.prompt;
  // Read out as a primitive, deliberately: the effect below must depend on the *value*, not
  // on the request object, which is a fresh snapshot on every store read — depending on that
  // would re-select the field under the operator's cursor mid-typing. `undefined` here also
  // *is* the discriminator, since a prompt always carries a string.
  const promptInitial = prompt?.initial;

  useEffect(() => {
    if (id === undefined) return;
    const previousFocus = document.activeElement as HTMLElement | null;
    if (promptInitial !== undefined) {
      setText(promptInitial);
      // Selected, not just focused: a rename starts from the current name far more often
      // than it appends to it, so typing should replace.
      inputRef.current?.focus();
      inputRef.current?.select();
    } else {
      confirmRef.current?.focus();
    }
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        answerConfirm(id, false);
      }
      if (event.key === "Tab") {
        const controls = Array.from(cardRef.current?.querySelectorAll<HTMLElement>(
          "button:not(:disabled), input:not(:disabled), [tabindex='0']",
        ) ?? []);
        const first = controls[0];
        const last = controls.at(-1);
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault(); last?.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault(); first?.focus();
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      if (previousFocus?.isConnected) previousFocus.focus();
    };
  }, [id, promptInitial]);

  if (!request) return null;

  return (
    <div className="confirm-layer">
      <div className="confirm-backdrop" onClick={() => answerConfirm(request.id, false)} />
      <div
        ref={cardRef}
        className="confirm-card"
        role="alertdialog"
        aria-modal="true"
        aria-label={request.title}
      >
        <h3>{request.title}</h3>
        {request.message && <p>{request.message}</p>}
        {prompt && (
          <input
            ref={inputRef}
            className="confirm-input"
            type={prompt.numeric ? "number" : "text"}
            value={text}
            placeholder={prompt.placeholder}
            aria-label={request.title}
            onChange={(event) => setText(event.target.value)}
            onKeyDown={(event) => {
              if (event.key !== "Enter") return;
              event.preventDefault();
              answerConfirm(request.id, true, text);
            }}
          />
        )}
        <div className="confirm-actions">
          <button type="button" onClick={() => answerConfirm(request.id, false)}>
            {request.cancelLabel ?? "Hủy"}
          </button>
          {request.alternateLabel && (
            <button type="button" onClick={() => answerConfirm(request.id, false, "discard")}>
              {request.alternateLabel}
            </button>
          )}
          <button
            ref={confirmRef}
            type="button"
            className={request.danger ? "danger" : "primary"}
            onClick={() => answerConfirm(request.id, true, text)}
          >
            {request.confirmLabel ?? "Tiếp tục"}
          </button>
        </div>
      </div>
    </div>
  );
}
