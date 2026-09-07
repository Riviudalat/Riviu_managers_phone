import { useLayoutEffect, useRef, type ReactNode } from "react";
import { useConfirmRequest } from "../../confirmStore";
import { createPortal } from "react-dom";
import { X } from "lucide-react";

export function PublishDialog({
  title,
  children,
  onClose,
  actions,
  wide = false,
  isOpen = true,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  actions?: ReactNode;
  wide?: boolean;
  isOpen?: boolean;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const confirmation = useConfirmRequest();
  // Native modal dialogs make the rest of the document inert. Temporarily leave
  // the top layer while the shared confirmation queue owns focus (save or Post).
  const show = isOpen && confirmation === null;
  useLayoutEffect(() => {
    const node = ref.current,
      previous = document.activeElement as HTMLElement | null;
    if (!show) {
      node?.close();
      return;
    }
    node?.showModal();
    return () => {
      node?.close();
      previous?.focus?.({ preventScroll: true });
    };
  }, [show]);
  return createPortal(
    <dialog
      ref={ref}
      className={`publish-dialog ${wide ? "is-wide" : ""}`}
      aria-label={title}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
    >
      <header>
        <h2>{title}</h2>
        <button
          type="button"
          className="ghost icon-only"
          aria-label="Đóng"
          onClick={onClose}
        >
          <X size={18} />
        </button>
      </header>
      <div className="publish-dialog-body">{children}</div>
      {actions && <footer>{actions}</footer>}
    </dialog>,
    document.body,
  );
}
