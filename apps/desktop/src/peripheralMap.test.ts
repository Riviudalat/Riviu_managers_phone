import { describe, expect, it } from "vitest";

import {
  defaultGamepadBindings,
  GAMEPAD_BUTTON,
  REFERENCE,
  resolveButtonAction,
  risingEdges,
  toReference,
  type GamepadBinding,
} from "./peripheralMap";

describe("defaultGamepadBindings", () => {
  it("maps the face buttons to coordinate-free nav keys", () => {
    const bindings = defaultGamepadBindings();
    expect(resolveButtonAction(bindings, GAMEPAD_BUTTON.A)).toEqual({ kind: "key", key: "home" });
    expect(resolveButtonAction(bindings, GAMEPAD_BUTTON.B)).toEqual({ kind: "key", key: "back" });
    expect(resolveButtonAction(bindings, GAMEPAD_BUTTON.X)).toEqual({ kind: "key", key: "recents" });
  });

  it("maps the D-pad to opposing swipes on the reference grid", () => {
    const bindings = defaultGamepadBindings();
    const up = resolveButtonAction(bindings, GAMEPAD_BUTTON.DPAD_UP);
    const down = resolveButtonAction(bindings, GAMEPAD_BUTTON.DPAD_DOWN);
    // Up scrolls the finger upward (start low, end high); down is its mirror.
    expect(up).toMatchObject({ kind: "swipe", fy1: 0.7, fy2: 0.3 });
    expect(down).toMatchObject({ kind: "swipe", fy1: 0.3, fy2: 0.7 });
  });
});

describe("resolveButtonAction", () => {
  it("returns null for an unbound button", () => {
    expect(resolveButtonAction(defaultGamepadBindings(), GAMEPAD_BUTTON.START)).toBeNull();
  });

  it("takes the first binding when a button is bound twice", () => {
    const bindings: GamepadBinding[] = [
      { button: 0, label: "first", action: { kind: "key", key: "home" } },
      { button: 0, label: "second", action: { kind: "key", key: "back" } },
    ];
    expect(resolveButtonAction(bindings, 0)).toEqual({ kind: "key", key: "home" });
  });
});

describe("risingEdges", () => {
  it("fires a freshly pressed button once", () => {
    expect(risingEdges([false, false], [true, false])).toEqual([0]);
  });

  it("does not re-fire a held button", () => {
    expect(risingEdges([true, false], [true, true])).toEqual([1]);
  });

  it("ignores releases", () => {
    expect(risingEdges([true, true], [false, true])).toEqual([]);
  });

  it("treats a newly-appearing index as a press only if it is down", () => {
    // prev reported 1 button, curr reports 3.
    expect(risingEdges([false], [false, true, false])).toEqual([1]);
  });
});

describe("toReference", () => {
  it("scales a fraction onto the reference grid", () => {
    expect(toReference(0.5)).toBe(REFERENCE / 2);
    expect(toReference(0)).toBe(0);
    expect(toReference(1)).toBe(REFERENCE);
  });

  it("clamps out-of-range fractions", () => {
    expect(toReference(-0.5)).toBe(0);
    expect(toReference(2)).toBe(REFERENCE);
  });
});
