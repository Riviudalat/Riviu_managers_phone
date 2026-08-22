/**
 * Macro record & replay (feature A8, xiaowei "录制动作 / execute action").
 *
 * A macro is an ordered list of the same actions `group_input` already fans out — a tap or
 * swipe at a point in a reference image, a hardware key, or a wait. Recording captures what
 * the operator does on one device (in the control overlay); replay re-issues each step to a
 * whole selection through `group_input`, which maps the reference-image point onto every
 * device's own screen and applies the group-sync delay/offset (A1) for free.
 *
 * Coordinates are stored in the *reference image space* (the overlay's encoded frame size,
 * `iw`×`ih`) exactly as `group_input` expects, so no per-device math lives here — each
 * device's session maps the point to its own resolution at replay time.
 *
 * This module is pure and unit-tested. Recording state and persistence live in
 * `macroStore.ts`; execution lives in the Group Tools "Macro" tab.
 */
import type { HardwareKey } from "./types";

export type MacroStep =
  | { kind: "tap"; x: number; y: number; iw: number; ih: number; afterMs: number }
  | {
      kind: "swipe";
      x: number;
      y: number;
      toX: number;
      toY: number;
      iw: number;
      ih: number;
      afterMs: number;
    }
  | { kind: "key"; key: HardwareKey; afterMs: number }
  | { kind: "wait"; afterMs: number };

export interface Macro {
  id: string;
  name: string;
  steps: MacroStep[];
}

/// Enough for a real routine, capped so a stuck recorder cannot grow an unbounded array in
/// localStorage.
export const MAX_MACRO_STEPS = 500;
/// A single inter-step wait is clamped to this so one accidental long pause during recording
/// does not bake a multi-minute freeze into every replay.
export const MAX_STEP_GAP_MS = 10_000;

/// Clamp a recorded inter-step gap into `[0, MAX_STEP_GAP_MS]`, rounding sub-millisecond
/// noise away. A negative gap (clock skew) becomes 0.
export function clampGap(ms: number): number {
  if (!Number.isFinite(ms) || ms < 0) return 0;
  return Math.min(Math.round(ms), MAX_STEP_GAP_MS);
}

/// Repeat the step list `loops` times for replay. `loops` below 1 is treated as 1 (a single
/// pass) — infinite looping is the caller's concern, guarded by a stop signal, never encoded
/// as a giant array here.
export function expand(steps: MacroStep[], loops: number): MacroStep[] {
  const n = Number.isFinite(loops) && loops >= 1 ? Math.floor(loops) : 1;
  const out: MacroStep[] = [];
  for (let i = 0; i < n; i += 1) out.push(...steps);
  return out;
}

/// Total wall-clock the waits alone will cost for a replay (excludes the time each action
/// itself takes on the device). Used to warn before a long run.
export function totalWaitMs(steps: MacroStep[], loops: number): number {
  const once = steps.reduce((sum, step) => sum + step.afterMs, 0);
  return once * (Number.isFinite(loops) && loops >= 1 ? Math.floor(loops) : 1);
}

/// A short human label for a step, for the recorded-step list in the UI.
export function stepSummary(step: MacroStep): string {
  switch (step.kind) {
    case "tap":
      return `Chạm (${Math.round(step.x)}, ${Math.round(step.y)})`;
    case "swipe":
      return `Vuốt (${Math.round(step.x)},${Math.round(step.y)}) → (${Math.round(
        step.toX,
      )},${Math.round(step.toY)})`;
    case "key":
      return `Phím ${step.key}`;
    case "wait":
      return `Chờ ${step.afterMs}ms`;
  }
}
