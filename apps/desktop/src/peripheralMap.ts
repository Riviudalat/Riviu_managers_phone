/**
 * Peripheral input → fleet action mapping (Giai đoạn D, xiaowei "外设": HID/gamepad routing).
 *
 * Pure and input-source-agnostic so it is unit-testable: the UI reads a controller through
 * the browser's Web Gamepad API (WebView2 exposes it natively — no host driver, no native
 * crate) and hands button state here; this decides which fleet gesture each button means.
 *
 * Actions are expressed against a **reference grid** ({@link REFERENCE}×{@link REFERENCE}), not
 * device pixels, so one binding drives phones of different resolutions — `group_input` scales
 * the reference rectangle onto each screen the same way the overlay does (`imageW/imageH`).
 * Hardware keys carry no coordinates and are exact everywhere.
 */
import type { HardwareKey } from "./types";

/** The square coordinate space bindings are authored in; `group_input` scales it per device. */
export const REFERENCE = 1000;

export type PeripheralAction =
  | { kind: "key"; key: HardwareKey }
  | { kind: "tap"; fx: number; fy: number }
  | { kind: "swipe"; fx1: number; fy1: number; fx2: number; fy2: number; durationMs: number }
  | { kind: "macro"; name: string };

export interface GamepadBinding {
  /** W3C "standard" gamepad button index. */
  button: number;
  label: string;
  action: PeripheralAction;
}

/** Button indices from the W3C Standard Gamepad mapping. */
export const GAMEPAD_BUTTON = {
  A: 0,
  B: 1,
  X: 2,
  Y: 3,
  LB: 4,
  RB: 5,
  BACK: 8,
  START: 9,
  DPAD_UP: 12,
  DPAD_DOWN: 13,
  DPAD_LEFT: 14,
  DPAD_RIGHT: 15,
} as const;

/**
 * Coordinate-free-ish defaults that work on any phone without calibration: the face buttons
 * are Android nav keys (exact everywhere), and the D-pad flicks the feed via reference-grid
 * swipes (scaled per device). The operator can rebind to absolute taps or macros for games.
 */
export function defaultGamepadBindings(): GamepadBinding[] {
  return [
    { button: GAMEPAD_BUTTON.A, label: "A → Home", action: { kind: "key", key: "home" } },
    { button: GAMEPAD_BUTTON.B, label: "B → Back", action: { kind: "key", key: "back" } },
    { button: GAMEPAD_BUTTON.X, label: "X → Đa nhiệm", action: { kind: "key", key: "recents" } },
    {
      button: GAMEPAD_BUTTON.DPAD_UP,
      label: "D-pad ↑ → vuốt lên",
      action: { kind: "swipe", fx1: 0.5, fy1: 0.7, fx2: 0.5, fy2: 0.3, durationMs: 250 },
    },
    {
      button: GAMEPAD_BUTTON.DPAD_DOWN,
      label: "D-pad ↓ → vuốt xuống",
      action: { kind: "swipe", fx1: 0.5, fy1: 0.3, fx2: 0.5, fy2: 0.7, durationMs: 250 },
    },
    {
      button: GAMEPAD_BUTTON.DPAD_LEFT,
      label: "D-pad ← → vuốt trái",
      action: { kind: "swipe", fx1: 0.3, fy1: 0.5, fx2: 0.7, fy2: 0.5, durationMs: 250 },
    },
    {
      button: GAMEPAD_BUTTON.DPAD_RIGHT,
      label: "D-pad → → vuốt phải",
      action: { kind: "swipe", fx1: 0.7, fy1: 0.5, fx2: 0.3, fy2: 0.5, durationMs: 250 },
    },
  ];
}

/** The action bound to a button, or null if unbound. First binding wins on a duplicate. */
export function resolveButtonAction(
  bindings: GamepadBinding[],
  button: number,
): PeripheralAction | null {
  return bindings.find((binding) => binding.button === button)?.action ?? null;
}

/**
 * Indices that went from not-pressed to pressed between two polls, so a held button fires
 * once rather than every frame. Tolerates the two arrays differing in length — a controller
 * can report a different button count between polls, and a newly-appearing index counts as a
 * press only if it is actually down.
 */
export function risingEdges(prev: boolean[], curr: boolean[]): number[] {
  const edges: number[] = [];
  for (let i = 0; i < curr.length; i += 1) {
    if (curr[i] && !prev[i]) edges.push(i);
  }
  return edges;
}

/** Map a fraction (0..1, clamped) onto the reference grid, as an integer. */
export function toReference(fraction: number): number {
  return Math.round(Math.max(0, Math.min(1, fraction)) * REFERENCE);
}
