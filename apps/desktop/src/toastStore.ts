import { useSyncExternalStore } from "react";

import { describeError } from "./describeError";

export type ToastKind = "ok" | "warn" | "error" | "info";

export interface ToastRecord {
  id: number;
  kind: ToastKind;
  title: string;
  detail?: string;
  createdAt: number;
}

type Listener = () => void;

const listeners = new Set<Listener>();
/** Immutable snapshot: useSyncExternalStore requires a stable reference while unchanged. */
let toasts: ToastRecord[] = [];
let nextId = 1;

/** Newest first. The bounded activity history never covers the active workspace. */
const MAX_RECORDS = 100;

function emit() {
  for (const listener of listeners) listener();
}

function commit(next: ToastRecord[]) {
  toasts = next;
  emit();
}

export function dismissToast(id: number) {
  if (!toasts.some((toast) => toast.id === id)) return;
  commit(toasts.filter((toast) => toast.id !== id));
}

/** Append an entry to the operator-controlled activity history. */
export function pushToast(kind: ToastKind, title: string, detail?: string): number {
  const id = nextId++;
  const record: ToastRecord = {
    id,
    kind,
    title,
    detail: detail || undefined,
    createdAt: Date.now(),
  };
  commit([record, ...toasts].slice(0, MAX_RECORDS));
  return id;
}

export function clearToasts() {
  if (toasts.length === 0) return;
  commit([]);
}

/**
 * Report a failed action. Unknown throwables reach here as `Error`, Tauri
 * `CommandError` payloads, or plain strings — normalise them to one line so the
 * toast stays readable instead of printing `[object Object]`.
 */
export function toastError(title: string, cause: unknown) {
  pushToast("error", title, describeError(cause));
}


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

/** Test seam and activity-center action: drop the in-memory history. */
export function resetToasts() {
  commit([]);
  nextId = 1;
}
