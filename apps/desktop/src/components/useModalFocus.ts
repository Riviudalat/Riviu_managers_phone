import { useEffect, useId, useRef } from "react";

const modalStack: HTMLElement[] = [];
const FOCUSABLE = 'button:not(:disabled), [href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), summary, [tabindex]:not([tabindex="-1"])';

/** Focus ownership for a mounted modal; nested confirmations keep their own keyboard. */
export function useModalFocus<T extends HTMLElement>(onClose: () => void, enabled = true, options?: {
  initialFocus?: () => HTMLElement | null;
  restoreFocus?: () => HTMLElement | null;
}) {
  const ref = useRef<T>(null);
  const scopeId = useId();
  const close = useRef(onClose);
  close.current = onClose;
  const focusOptions = useRef(options);
  focusOptions.current = options;
  const previousFocus = useRef(typeof document === "undefined" ? null : document.activeElement);

  useEffect(() => {
    if (!enabled) return;
    const dialog = ref.current;
    if (!dialog) return;
    const previous = dialog.contains(document.activeElement) ? previousFocus.current : document.activeElement;
    dialog.dataset.modalFocusScope = scopeId;
    modalStack.push(dialog);
    const portals = () => Array.from(document.querySelectorAll<HTMLElement>("[data-modal-focus-owner]"))
      .filter((portal) => portal.dataset.modalFocusOwner === scopeId);
    const controlsIn = (root: HTMLElement) => Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE))
      .filter((element) => {
        if (element.getAttribute("tabindex") === "-1" || element.closest('[hidden], [inert], [aria-hidden="true"]')) return false;
        for (let parent: HTMLElement | null = element; parent && parent !== dialog; parent = parent.parentElement) {
          const style = getComputedStyle(parent);
          if (style.visibility === "hidden" || style.display === "none") return false;
          if (parent instanceof HTMLDetailsElement && !parent.open && !parent.querySelector("summary")?.contains(element)) return false;
        }
        return true;
      })
      .sort((left, right) => left.compareDocumentPosition(right) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1);
    const focusable = () => {
      const items = controlsIn(dialog);
      for (const portal of portals()) {
        const anchor = document.getElementById(portal.dataset.modalFocusAnchor ?? "");
        const index = anchor ? items.indexOf(anchor) : -1;
        // A flyout follows its owning menu row, not unrelated controls between portal roots.
        items.splice(index < 0 ? items.length : index + 1, 0, ...controlsIn(portal));
      }
      return items;
    };
    const initial = focusOptions.current?.initialFocus?.();
    if (initial && dialog.contains(initial)) initial.focus();
    else if (!dialog.contains(document.activeElement)) (focusable()[0] ?? dialog).focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || modalStack.at(-1) !== dialog) return;
      const active = document.activeElement;
      const activeModal = active instanceof HTMLElement ? active.closest('[role="dialog"][aria-modal="true"], [role="alertdialog"]') : null;
      if (activeModal && activeModal !== dialog) return;
      const ownedPortals = portals();
      if (event.key === "Escape") {
        // Owned flyouts handle Escape locally and return focus to their menu row.
        if (ownedPortals.some((portal) => portal.contains(active) || document.getElementById(portal.dataset.modalFocusAnchor ?? "") === active)) return;
        event.preventDefault();
        event.stopPropagation();
        close.current();
        return;
      }
      if (event.key !== "Tab") return;
      const items = focusable();
      const first = items[0] ?? dialog;
      const last = items.at(-1) ?? dialog;
      if (ownedPortals.length) {
        event.preventDefault();
        const index = active instanceof HTMLElement ? items.indexOf(active) : -1;
        const next = index < 0 ? (event.shiftKey ? last : first) : items[(index + (event.shiftKey ? -1 : 1) + items.length) % items.length];
        (next ?? dialog).focus();
        return;
      }
      if (!dialog.contains(active) || active === dialog || (!event.shiftKey && active === last)) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
      } else if (event.shiftKey && active === first) {
        event.preventDefault();
        last.focus();
      }
    };
    document.addEventListener("keydown", onKeyDown, true);
    return () => {
      modalStack.splice(modalStack.indexOf(dialog), 1);
      delete dialog.dataset.modalFocusScope;
      document.removeEventListener("keydown", onKeyDown, true);
      const previousUsable = previous instanceof HTMLElement && previous.isConnected &&
        previous !== document.body && !previous.closest('[hidden], [inert]');
      if (previousUsable) previous.focus();
      else focusOptions.current?.restoreFocus?.()?.focus();
    };
  }, [enabled, scopeId]);

  return ref;
}
