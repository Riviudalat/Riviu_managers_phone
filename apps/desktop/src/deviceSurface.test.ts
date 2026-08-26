import { describe, expect, it } from "vitest";

import { surfaceDeparted } from "./deviceSurface";

const fleet = [{ udid: "ce0717171c2a64d50d" }, { udid: "10969614" }];

describe("surfaceDeparted", () => {
  it("keeps a surface open while its phone is in the fleet", () => {
    expect(surfaceDeparted(fleet, "10969614")).toBe(false);
  });

  it("closes a surface whose phone has left a fleet that still has others", () => {
    // The real case: one phone unplugged, the rest still there.
    expect(surfaceDeparted(fleet, "ce0417145199e0490c")).toBe(true);
  });

  it("does NOT close anything when the roster is empty", () => {
    // **The load-bearing case.** A roster is empty at boot before the first scan lands, and
    // again whenever a scan fails — a restarting adb server answers once with nothing. Treating
    // that as a departure makes the app close every panel during a blip it recovers from a
    // second later, which is a worse bug than the one this predicate fixes.
    expect(surfaceDeparted([], "10969614")).toBe(false);
  });

  it("has nothing to close when no surface is open", () => {
    expect(surfaceDeparted(fleet, null)).toBe(false);
    expect(surfaceDeparted([], null)).toBe(false);
  });
});
