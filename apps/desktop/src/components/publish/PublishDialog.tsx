import { useLayoutEffect, useRef, type ReactNode } from "react";
import { useConfirmRequest } from "../../confirmStore";
import { createPortal } from "react-dom";
import { X } from "lucide-react";

function focusVisible(node: HTMLElement | null | undefined) {
  if (!node?.isConnected || node.closest('[hidden], [inert], dialog:not([open]), details:not([open]) > :not(summary)')
    || node.matches(":disabled") || getComputedStyle(node).display === "none" || getComputedStyle(node).visibility === "hidden") return false;
  node.focus({ preventScroll: true });
  return document.activeElement === node;
}

export function PublishDialog({
  title,
  children,
  onClose,
  actions,
  wide = false,
  isOpen = true,
  returnFocus,
  fallbackFocus,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  actions?: ReactNode;
  wide?: boolean;
  isOpen?: boolean;
  returnFocus?: () => HTMLElement | null;
  fallbackFocus?: () => HTMLElement | null;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const origin = useRef<HTMLElement | null>(null);
  const mounted = useRef(false);
  const focusContext = useRef({ returnFocus, fallbackFocus });
  const confirmation = useConfirmRequest();
  // Nhường native top-layer cho hàng đợi confirm chung, không tạo focus trap thứ hai.
  const show = isOpen && confirmation === null;
  useLayoutEffect(() => { focusContext.current = { returnFocus, fallbackFocus }; });
  useLayoutEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      // Chờ commit xong để biết trigger còn trong nguồn/bộ lọc hiện hành hay không.
      queueMicrotask(() => {
        if (mounted.current || document.querySelector('.confirm-layer, dialog[open]')) return;
        const context = focusContext.current;
        if (!focusVisible(context.returnFocus ? context.returnFocus() : origin.current)) focusVisible(context.fallbackFocus?.());
      });
    };
  }, []);
  useLayoutEffect(() => {
    const node = ref.current;
    if (!show) {
      node?.close();
      if (!isOpen && !confirmation) {
        const context = focusContext.current;
        if (!focusVisible(context.returnFocus ? context.returnFocus() : origin.current)) focusVisible(context.fallbackFocus?.());
      }
      return;
    }
    if (!origin.current) origin.current = document.activeElement as HTMLElement | null;
    node?.showModal();
    return () => { node?.close(); };
  }, [show, isOpen, confirmation]);
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
        <button type="button" className="ghost icon-only" aria-label="Đóng" onClick={onClose}>
          <X size={18} />
        </button>
      </header>
      <div className="publish-dialog-body">{children}</div>
      {actions && <footer>{actions}</footer>}
    </dialog>,
    document.body,
  );
}
