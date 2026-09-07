import type { TargetRef } from "./types";

const PREFIX = "riviu.form-draft.v1.";

/** Local editor state only. Never pass credentials, approvals or execution evidence here. */
export function readFormDraft<T>(id: string, decode: (value: unknown) => T): T | null {
  try {
    const raw = localStorage.getItem(PREFIX + id);
    if (raw === null) return null;
    const envelope = JSON.parse(raw);
    if (envelope?.schemaVersion !== 1) return null;
    return decode(envelope.value);
  } catch { return null; }
}

export function writeFormDraft(id: string, value: unknown): void {
  localStorage.setItem(PREFIX + id, JSON.stringify({ schemaVersion: 1, value }));
}

/** Merge known fields only. Additive UI revisions inherit their new defaults. */
export function restoreFormShape<T>(value: unknown, fallback: T): T {
  if (value === null || value === undefined) return fallback;
  if (Array.isArray(fallback)) return (Array.isArray(value) && value.every(item => typeof item === "string") ? value : fallback) as T;
  if (typeof fallback === "object" && fallback !== null) {
    if (typeof value !== "object" || Array.isArray(value)) return fallback;
    return Object.fromEntries(Object.entries(fallback).map(([key, initial]) =>
      [key, restoreFormShape((value as Record<string, unknown>)[key], initial)])) as T;
  }
  return typeof value === typeof fallback ? value as T : fallback;
}

export function readTargetDraft(id: string, fallback: TargetRef): TargetRef {
  return readFormDraft(id, value => {
    if (!value || typeof value !== "object" || !("type" in value)) return fallback;
    if (value.type === "all") return { type: "all" };
    if (value.type === "group" && "groupId" in value && typeof value.groupId === "string") return { type: "group", groupId: value.groupId };
    if (value.type === "explicit" && "udids" in value && Array.isArray(value.udids)
      && value.udids.every(id => typeof id === "string")) return { type: "explicit", udids: value.udids };
    return fallback;
  }) ?? fallback;
}
