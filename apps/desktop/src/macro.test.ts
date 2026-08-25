import { describe, it, expect } from "vitest";
import {
  clampGap,
  expand,
  MAX_STEP_GAP_MS,
  stepSummary,
  totalWaitMs,
  type MacroStep,
} from "./macro";

const tap = (afterMs = 0): MacroStep => ({ kind: "tap", x: 10, y: 20, iw: 100, ih: 200, afterMs });

describe("clampGap", () => {
  it("floors negatives and non-finite to 0, rounds, and caps", () => {
    expect(clampGap(-5)).toBe(0);
    expect(clampGap(Number.NaN)).toBe(0);
    expect(clampGap(123.6)).toBe(124);
    expect(clampGap(MAX_STEP_GAP_MS + 5000)).toBe(MAX_STEP_GAP_MS);
  });
});

describe("expand", () => {
  it("repeats the step list loops times", () => {
    const steps = [tap(), tap()];
    expect(expand(steps, 3)).toHaveLength(6);
  });

  it("treats loops < 1 as a single pass", () => {
    const steps = [tap()];
    expect(expand(steps, 0)).toHaveLength(1);
    expect(expand(steps, -4)).toHaveLength(1);
  });

  it("keeps an empty macro empty", () => {
    expect(expand([], 5)).toEqual([]);
  });
});

describe("totalWaitMs", () => {
  it("sums the waits across loops", () => {
    const steps = [tap(100), tap(250)];
    expect(totalWaitMs(steps, 2)).toBe(700);
  });
});

describe("stepSummary", () => {
  it("describes each kind", () => {
    expect(stepSummary(tap())).toContain("Chạm");
    expect(
      stepSummary({ kind: "swipe", x: 1, y: 2, toX: 3, toY: 4, iw: 9, ih: 9, afterMs: 0 }),
    ).toContain("Vuốt");
    expect(stepSummary({ kind: "key", key: "home", afterMs: 0 })).toContain("home");
    expect(stepSummary({ kind: "wait", afterMs: 500 })).toContain("500");
  });
});
