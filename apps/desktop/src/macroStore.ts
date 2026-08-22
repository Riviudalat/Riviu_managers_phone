/**
 * Recording buffer + saved-macro persistence for feature A8.
 *
 * `FocusStream` calls `recordTap/recordSwipe/recordKey` as the operator drives one device;
 * they no-op unless recording is armed, so the control overlay is unchanged when the feature
 * is off. The Group Tools "Macro" tab arms recording, saves the buffer as a named macro
 * (localStorage), and replays a saved macro across the selection.
 */
import { useSyncExternalStore } from "react";
import type { HardwareKey } from "./types";
import { clampGap, MAX_MACRO_STEPS, type Macro, type MacroStep } from "./macro";

const KEY = "riviu.macros";

let recording = false;
let buffer: MacroStep[] = [];
let lastAt = 0;
const listeners = new Set<() => void>();

function emit(): void {
  for (const l of listeners) l();
}

function loadSaved(): Macro[] {
  try {
    const raw = window.localStorage.getItem(KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (m): m is Macro =>
        typeof m === "object" &&
        m !== null &&
        typeof (m as Macro).id === "string" &&
        typeof (m as Macro).name === "string" &&
        Array.isArray((m as Macro).steps),
    );
  } catch {
    return [];
  }
}

let saved: Macro[] = loadSaved();

function persist(): void {
  try {
    window.localStorage.setItem(KEY, JSON.stringify(saved));
  } catch {
    // Persistence is a convenience; losing it must not break recording or replay.
  }
}

// --- recording, called from FocusStream ---------------------------------------------------

export function isRecordingMacro(): boolean {
  return recording;
}

export function startRecording(): void {
  recording = true;
  buffer = [];
  lastAt = performanceNow();
  emit();
}

/** Stop and return the recorded steps (also stays available via {@link recordedSteps}). */
export function stopRecording(): MacroStep[] {
  recording = false;
  emit();
  return buffer;
}

export function recordedSteps(): MacroStep[] {
  return buffer;
}

export function clearRecording(): void {
  buffer = [];
  lastAt = performanceNow();
  emit();
}

function performanceNow(): number {
  // performance.now avoids the wall-clock jumps Date.now can take; falls back if absent.
  try {
    return performance.now();
  } catch {
    return 0;
  }
}

/** Append a step, first charging the pause since the previous step to that previous step. */
function append(step: MacroStep): void {
  if (!recording || buffer.length >= MAX_MACRO_STEPS) return;
  const now = performanceNow();
  const gap = clampGap(now - lastAt);
  lastAt = now;
  const next = [...buffer];
  if (next.length > 0) {
    next[next.length - 1] = { ...next[next.length - 1], afterMs: gap };
  }
  next.push(step);
  buffer = next;
  emit();
}

export function recordTap(x: number, y: number, iw: number, ih: number): void {
  append({ kind: "tap", x, y, iw, ih, afterMs: 0 });
}

export function recordSwipe(
  x: number,
  y: number,
  toX: number,
  toY: number,
  iw: number,
  ih: number,
): void {
  append({ kind: "swipe", x, y, toX, toY, iw, ih, afterMs: 0 });
}

export function recordKey(key: HardwareKey): void {
  append({ kind: "key", key, afterMs: 0 });
}

// --- saved macros ------------------------------------------------------------------------

function newId(): string {
  try {
    return crypto.randomUUID();
  } catch {
    return `macro-${performanceNow()}-${buffer.length}`;
  }
}

export function savedMacros(): Macro[] {
  return saved;
}

export function saveMacro(name: string, steps: MacroStep[]): Macro | null {
  if (!steps.length) return null;
  const macro: Macro = { id: newId(), name: name.trim() || `Macro ${saved.length + 1}`, steps };
  saved = [...saved, macro];
  persist();
  emit();
  return macro;
}

export function deleteMacro(id: string): void {
  saved = saved.filter((m) => m.id !== id);
  persist();
  emit();
}

// --- react binding -----------------------------------------------------------------------

function subscribe(cb: () => void): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

/** A stable snapshot object would be ideal, but the pieces the UI needs are read
 * individually through these hooks, each with its own getSnapshot. */
export function useMacroRecording(): boolean {
  return useSyncExternalStore(subscribe, isRecordingMacro);
}

export function useRecordedSteps(): MacroStep[] {
  return useSyncExternalStore(subscribe, recordedSteps);
}

export function useSavedMacros(): Macro[] {
  return useSyncExternalStore(subscribe, savedMacros);
}
