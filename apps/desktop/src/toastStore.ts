import { useSyncExternalStore } from "react";

import { describeError } from "./describeError";

export type ToastKind = "ok" | "warn" | "error" | "info";

export interface ToastRecord {
  id: number;
  kind: ToastKind;
  title: string;
  detail?: string;
}

type Listener = () => void;

const listeners = new Set<Listener>();
const timers = new Map<number, ReturnType<typeof setTimeout>>();
/** Immutable snapshot: useSyncExternalStore requires a stable reference while unchanged. */
let toasts: ToastRecord[] = [];
let nextId = 1;

/** Newest first, and bounded so a burst of device errors cannot fill the screen. */
const MAX_VISIBLE = 4;

const LIFETIME_MS: Record<ToastKind, number> = {
  ok: 4000,
  info: 4500,
  warn: 6500,
  error: 9000,
};

function emit() {
  for (const listener of listeners) listener();
}

function commit(next: ToastRecord[]) {
  toasts = next;
  emit();
}

export function dismissToast(id: number) {
  const timer = timers.get(id);
  if (timer !== undefined) {
    clearTimeout(timer);
    timers.delete(id);
  }
  if (!toasts.some((toast) => toast.id === id)) return;
  commit(toasts.filter((toast) => toast.id !== id));
}

/** Show a transient notification. Returns its id so callers can dismiss early. */
export function pushToast(kind: ToastKind, title: string, detail?: string): number {
  const id = nextId++;
  const record: ToastRecord = { id, kind, title, detail: detail || undefined };
  const kept = [record, ...toasts].slice(0, MAX_VISIBLE);
  // Drop timers for toasts pushed off the end of the stack.
  for (const toast of toasts) {
    if (!kept.includes(toast)) dismissTimerOnly(toast.id);
  }
  commit(kept);
  timers.set(
    id,
    setTimeout(() => dismissToast(id), LIFETIME_MS[kind]),
  );
  return id;
}

function dismissTimerOnly(id: number) {
  const timer = timers.get(id);
  if (timer === undefined) return;
  clearTimeout(timer);
  timers.delete(id);
}

/**
 * Report a failed action. Unknown throwables reach here as `Error`, Tauri
 * `CommandError` payloads, or plain strings — normalise them to one line so the
 * toast stays readable instead of printing `[object Object]`.
 */
export function toastError(title: string, cause: unknown) {
  pushToast("error", title, describeError(cause));
}

export { describeError } from "./describeError";

export function useToasts(): ToastRecord[] {
  return useSyncExternalStore(
    (onStoreChange) => {
      listeners.add(onStoreChange);
      return () => listeners.delete(onStoreChange);
    },
    () => toasts,
    () => toasts,
  );
}

/** Test seam — drops every queued toast and its timer. */
export function resetToasts() {
  for (const timer of timers.values()) clearTimeout(timer);
  timers.clear();
  commit([]);
}
