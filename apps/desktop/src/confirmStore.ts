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

/**
 * A dialog that asks for a value rather than a yes/no — renaming a phone, numbering it.
 *
 * On the same queue as the confirms on purpose, rather than a second modal host: two modal
 * layers that do not know about each other can open at once, and the second one lands on top
 * of a dialog somebody is already answering.
 */
export interface PromptRequest extends ConfirmRequest {
  /** Pre-filled, and selected, so replacing the whole value takes no extra clicks. */
  initial?: string;
  placeholder?: string;
  /**
   * Renders a number input. Not a validator: what a number *means* is the caller's business
   * (a phone numbered 0 is nonsense, 21 is a shelf position), so the caller checks what it
   * gets back. This only stops letters being typed into it.
   */
  numeric?: boolean;
}

interface PendingConfirm extends ConfirmRequest {
  id: number;
  /**
   * Set for a prompt, absent for a plain confirm — which is how the host tells them apart,
   * and why `requestPrompt` normalises the optional fields here rather than at render time.
   */
  prompt?: { initial: string; placeholder?: string; numeric: boolean };
  settle: (ok: boolean, text: string) => void;
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

function enqueue(pending: PendingConfirm) {
  if (active === null) {
    active = pending;
    emit();
  } else {
    queue.push(pending);
  }
}

/**
 * Ask the operator to confirm a consequential action. Replaces `window.confirm`
 * so the prompt matches the app instead of the OS, and so the copy can name the
 * real consequence. Requests queue rather than overwrite each other.
 */
export function requestConfirm(request: ConfirmRequest): Promise<boolean> {
  return new Promise<boolean>((resolve) => {
    enqueue({ ...request, id: nextId++, settle: (ok) => resolve(ok) });
  });
}

/**
 * Ask the operator for one value.
 *
 * Resolves to the trimmed text, or `null` if they cancelled — so an empty answer and a
 * cancelled dialog stay distinguishable: `""` means "clear this", `null` means "never mind".
 * A rename that could not tell those apart would clear the alias every time somebody pressed
 * Escape.
 */
export function requestPrompt(request: PromptRequest): Promise<string | null> {
  const { initial, placeholder, numeric, ...rest } = request;
  return new Promise<string | null>((resolve) => {
    enqueue({
      ...rest,
      id: nextId++,
      prompt: { initial: initial ?? "", placeholder, numeric: numeric ?? false },
      settle: (ok, text) => resolve(ok ? text.trim() : null),
    });
  });
}

export function answerConfirm(id: number, answer: boolean, text = "") {
  if (active === null || active.id !== id) return;
  const { settle } = active;
  advance();
  settle(answer, text);
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
  for (const pending of pendings) pending.settle(false, "");
}
