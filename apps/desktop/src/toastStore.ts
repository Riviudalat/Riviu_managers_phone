import { useSyncExternalStore } from "react";

import { describeError, errorFacts } from "./describeError";

export type ToastKind = "ok" | "warn" | "error" | "info";

/**
 * Typed facts the producer already holds. Activity groups entries by `cause` and offers to
 * select exactly `udids`; nothing here is parsed back out of the human sentence.
 */
export interface ActivityMeta {
  /** Stable machine cause: a backend error code or a producer-owned key. */
  cause?: string;
  udid?: string;
  /** Every phone the entry is about, e.g. the failed share of a fan-out. */
  udids?: readonly string[];
  runId?: string;
}

export interface ToastRecord {
  id: number;
  kind: ToastKind;
  title: string;
  detail?: string;
  summary?: string;
  cause?: string;
  /** Distinct device IDs from `udid` and `udids`, in producer order. */
  udids: readonly string[];
  runId?: string;
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
export function pushToast(
  kind: ToastKind,
  title: string,
  detail?: string,
  summary?: string,
  meta?: ActivityMeta,
): number {
  const id = nextId++;
  const udids = [...new Set([meta?.udid, ...(meta?.udids ?? [])].filter((udid): udid is string => !!udid))];
  const record: ToastRecord = {
    id,
    kind,
    title,
    detail: detail || undefined,
    summary: summary || undefined,
    cause: meta?.cause || undefined,
    udids,
    runId: meta?.runId || undefined,
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
 * toast stays readable instead of printing `[object Object]`. The error's own typed code,
 * udid and runId fill whatever the caller did not already know.
 */
export function toastError(title: string, cause: unknown, meta?: ActivityMeta) {
  const facts = errorFacts(cause);
  pushToast("error", title, describeError(cause), undefined, {
    cause: meta?.cause ?? facts.code,
    udid: meta?.udid ?? facts.udid,
    udids: meta?.udids,
    runId: meta?.runId ?? facts.runId,
  });
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
