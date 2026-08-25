/**
 * Group-sync policy store (feature A1, ported from xiaowei `delaySync` / `delayOffset`).
 *
 * A single global policy — how much per-device delay and coordinate jitter to add when one
 * gesture fans out to a whole group — configured in Settings and read at each `group_input`
 * call. Persisted to localStorage like {@link ./zoom.ts} so it survives a restart. The Rust
 * side (`riviu_core::group_sync`) is the authority on how the policy is *applied*; this is
 * only how the operator edits and stores it.
 */
import { useSyncExternalStore } from "react";
import type { DelayPolicy, GroupSyncPolicy } from "./types";

const KEY = "riviu.groupSync";

export function defaultGroupSync(): GroupSyncPolicy {
  return { delay: { mode: "none" }, offset: { maxPx: 0 } };
}

/** Clamp to a non-negative integer; anything unparseable falls back to `fallback`. */
function nonNegInt(value: unknown, fallback: number): number {
  const n = Math.round(Number(value));
  return Number.isFinite(n) && n >= 0 ? n : fallback;
}

function normalizeDelay(raw: unknown): DelayPolicy {
  if (raw && typeof raw === "object" && "mode" in raw) {
    const mode = (raw as { mode: unknown }).mode;
    if (mode === "random") {
      const r = raw as { minMs?: unknown; maxMs?: unknown };
      return { mode: "random", minMs: nonNegInt(r.minMs, 0), maxMs: nonNegInt(r.maxMs, 0) };
    }
    if (mode === "staggered") {
      const r = raw as { stepMs?: unknown };
      return { mode: "staggered", stepMs: nonNegInt(r.stepMs, 0) };
    }
  }
  return { mode: "none" };
}

/** Coerce arbitrary stored/edited data into a valid policy — never trust the raw shape. */
export function normalizeGroupSync(raw: unknown): GroupSyncPolicy {
  const obj = (raw ?? {}) as { delay?: unknown; offset?: unknown };
  const maxPx = nonNegInt((obj.offset as { maxPx?: unknown } | undefined)?.maxPx, 0);
  return { delay: normalizeDelay(obj.delay), offset: { maxPx } };
}

function load(): GroupSyncPolicy {
  try {
    const raw = window.localStorage.getItem(KEY);
    if (raw === null) return defaultGroupSync();
    return normalizeGroupSync(JSON.parse(raw));
  } catch {
    return defaultGroupSync();
  }
}

function persist(policy: GroupSyncPolicy): void {
  try {
    window.localStorage.setItem(KEY, JSON.stringify(policy));
  } catch {
    // Persistence is a convenience; losing it must not break group input.
  }
}

let current: GroupSyncPolicy = load();
const listeners = new Set<() => void>();

/** The current policy. Stable reference between changes (safe for useSyncExternalStore). */
export function getGroupSync(): GroupSyncPolicy {
  return current;
}

export function setGroupSync(next: GroupSyncPolicy): void {
  current = normalizeGroupSync(next);
  persist(current);
  for (const listener of listeners) listener();
}

function subscribe(cb: () => void): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

/** Reactive read for editing UI. Handlers that only need the value at call time should use
 * {@link getGroupSync} instead, to avoid a stale closure and needless re-renders. */
export function useGroupSync(): GroupSyncPolicy {
  return useSyncExternalStore(subscribe, getGroupSync);
}

/** True when the policy does nothing — mirrors `GroupSyncPolicy::is_noop` on the Rust side. */
export function isNoopGroupSync(policy: GroupSyncPolicy): boolean {
  return (policy.delay?.mode ?? "none") === "none" && (policy.offset?.maxPx ?? 0) === 0;
}
