import { describe, expect, it } from "vitest";
import { focusLayout } from "./focusLayout";

describe("focus layout", () => {
  it("keeps the short edge and usable controls after landscape rotation", () => {
    const rotated = focusLayout(600, 288, 400, 1440, 900);
    expect(rotated.screenHeight).toBe(400);
    expect(rotated.screenWidth).toBeCloseTo(833.3333);
    expect(rotated.menuHeight).toBe(400);
  });
  for (const viewport of [[1440, 900], [900, 900], [820, 560], [390, 844]]) {
    for (const frame of [[288, 600], [600, 288]]) {
      it(`fits ${frame} in ${viewport} without distorting the frame`, () => {
        const result = focusLayout(frame[0], frame[1], 500, viewport[0], viewport[1]);
        expect(result.screenWidth / result.screenHeight).toBeCloseTo(frame[0] / frame[1]);
        expect(result.screenWidth + (result.stacked ? 0 : 220) + 26).toBeLessThanOrEqual(viewport[0]);
        expect((result.stacked ? result.screenHeight + result.menuHeight : Math.max(result.screenHeight, result.menuHeight)) + 26).toBeLessThanOrEqual(viewport[1]);
        expect(result.menuHeight).toBeGreaterThanOrEqual(250);
      });
    }
  }
});
