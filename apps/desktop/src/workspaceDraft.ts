import { useEffect, useLayoutEffect, useRef, useSyncExternalStore } from "react";
import { requestSaveChanges } from "./confirmStore";

export interface WorkspaceDraft {
  id: string;
  label: string;
  dirty: boolean;
  snapshotKey: string;
  save: () => Promise<boolean | void>;
  discard: () => void | Promise<void>;
  /** Separate from explicit profile save: autosaving must never grant execution consent. */
  autoSave?: () => Promise<boolean | void> | boolean | void;
  onAutoSaveError?: (error: unknown) => void;
}

type Entry = { read: () => WorkspaceDraft; acknowledged?: string; saving?: Promise<boolean> };
const drafts = new Map<string, Entry>();
const listeners = new Set<() => void>();
let pending: Promise<boolean> | null = null;
let pendingScope: string | null = null;
const emit = () => listeners.forEach((listener) => listener());
const isDirty = (entry: Entry) => {
  const draft = entry.read();
  return draft.dirty && entry.acknowledged !== draft.snapshotKey;
};

export function hasWorkspaceDrafts(): boolean {
  return [...drafts.values()].some(isDirty);
}

export function useWorkspaceDirty(): boolean {
  return useSyncExternalStore(
    (listener) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
    () => [...drafts.values()].some(entry => !entry.read().autoSave && isDirty(entry)),
    () => false,
  );
}

async function saveAutomatic(entry: Entry): Promise<boolean> {
  if (entry.saving) {
    const ok = await entry.saving;
    return ok && isDirty(entry) ? saveAutomatic(entry) : ok;
  }
  const draft = entry.read();
  if (!draft.autoSave || !isDirty(entry)) return true;
  const snapshot = draft.snapshotKey;
  entry.saving = (async () => {
    try {
      const saved = await draft.autoSave!();
      if (saved === false) return false;
      entry.acknowledged = snapshot;
      return true;
    } catch (error) {
      entry.read().onAutoSaveError?.(error);
      return false;
    }
  })();
  const ok = await entry.saving;
  entry.saving = undefined;
  emit();
  return ok && isDirty(entry) ? saveAutomatic(entry) : ok;
}

/** Only editable snapshots participate; polling and pending requests never make a draft dirty. */
export function useWorkspaceDraft(draft: WorkspaceDraft): void {
  const automatic = Boolean(draft.autoSave);
  const latest = useRef(draft);
  latest.current = draft;
  useLayoutEffect(() => {
    const entry: Entry = { read: () => latest.current };
    drafts.set(draft.id, entry);
    emit();
    return () => {
      if (drafts.get(draft.id) === entry) drafts.delete(draft.id);
      emit();
    };
  }, [draft.id]);
  useEffect(() => {
    const entry = drafts.get(draft.id);
    if (entry && (!draft.dirty || entry.acknowledged !== draft.snapshotKey)) {
      entry.acknowledged = undefined;
    }
    emit();
  }, [draft.id, draft.dirty, draft.snapshotKey]);
  useEffect(() => {
    if (!automatic || !draft.dirty) return;
    const timer = window.setTimeout(() => {
      const entry = drafts.get(draft.id);
      if (entry) void saveAutomatic(entry);
    }, 650);
    return () => window.clearTimeout(timer);
  }, [draft.id, draft.dirty, draft.snapshotKey, automatic]);
}

export function requestWorkspaceLeave(ids?: string[]): Promise<boolean> {
  const scope = ids ? [...ids].sort().join("\u0000") : "*";
  if (pending) {
    if (scope === pendingScope) return pending;
    return pending.then((proceed) => proceed ? requestWorkspaceLeave(ids) : false);
  }
  const entries = [...drafts.entries()]
    .filter(([id, entry]) => (!ids || ids.includes(id)) && isDirty(entry));
  if (!entries.length) return Promise.resolve(true);
  pendingScope = scope;
  pending = (async () => {
    for (const [, entry] of entries) {
      if (entry.read().autoSave && !(await saveAutomatic(entry))) return false;
    }
    const manual = entries.filter(([, entry]) => !entry.read().autoSave && isDirty(entry));
    if (!manual.length) return true;
    const choice = await requestSaveChanges(manual.map(([, entry]) => entry.read().label).join(", "));
    if (choice === "stay") return false;
    for (const [id, entry] of manual) {
      if (drafts.get(id) !== entry || !isDirty(entry)) continue;
      const draft = entry.read();
      const snapshot = draft.snapshotKey;
      try {
        const result = choice === "save" ? await draft.save() : await draft.discard();
        if (result === false) return false;
        // A response may finish after another keystroke. Never authorize dropping that edit.
        if (choice === "save" && entry.read().dirty && entry.read().snapshotKey !== snapshot) return false;
        entry.acknowledged = entry.read().snapshotKey;
      } catch {
        return false;
      }
    }
    emit();
    return ![...drafts.entries()].some(([id, entry]) => (!ids || ids.includes(id)) && isDirty(entry));
  })().finally(() => { pending = null; pendingScope = null; });
  return pending;
}
