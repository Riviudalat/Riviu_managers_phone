import { useSyncExternalStore } from "react";

export interface ConfirmRequest {
  title: string;
  /** Optional body copy. Say what will happen, not "are you sure". */
  message?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  /** Paints the confirm button as destructive (restore, uninstall, reboot…). */
  danger?: boolean;
}

interface PendingConfirm extends ConfirmRequest {
  id: number;
  resolve: (answer: boolean) => void;
}

type Listener = () => void;

const listeners = new Set<Listener>();
const queue: PendingConfirm[] = [];
/** Immutable snapshot for useSyncExternalStore; null when nothing is asked. */
let active: PendingConfirm | null = null;
let nextId = 1;

function emit() {
  for (const listener of listeners) listener();
}

function advance() {
  active = queue.shift() ?? null;
  emit();
}

/**
 * Ask the operator to confirm a consequential action. Replaces `window.confirm`
 * so the prompt matches the app instead of the OS, and so the copy can name the
 * real consequence. Requests queue rather than overwrite each other.
 */
export function requestConfirm(request: ConfirmRequest): Promise<boolean> {
  return new Promise<boolean>((resolve) => {
    const pending: PendingConfirm = { ...request, id: nextId++, resolve };
    if (active === null) {
      active = pending;
      emit();
    } else {
      queue.push(pending);
    }
  });
}

export function answerConfirm(id: number, answer: boolean) {
  if (active === null || active.id !== id) return;
  const { resolve } = active;
  advance();
  resolve(answer);
}

export function useConfirmRequest(): PendingConfirm | null {
  return useSyncExternalStore(
    (onStoreChange) => {
      listeners.add(onStoreChange);
      return () => listeners.delete(onStoreChange);
    },
    () => active,
    () => active,
  );
}

/** Test seam — answers everything pending with `false`. */
export function resetConfirms() {
  const pendings = active === null ? [] : [active, ...queue];
  queue.length = 0;
  active = null;
  emit();
  for (const pending of pendings) pending.resolve(false);
}
