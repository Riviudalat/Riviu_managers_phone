import { describe, expect, it } from "vitest";
import { markDeviceFrameLive, tileStreamStateView, type DeviceInfo } from "../types";

describe("tileStreamStateView", () => {
  it.each([
    ["live", "Live"],
    ["sampling", "Sampling"],
    ["parked", "Parked"],
    ["stale", "Stale"],
    ["error", "Error"],
  ] as const)("maps %s to a compact %s label", (state, label) => {
    expect(tileStreamStateView(state, false, false)).toEqual({ state, label });
  });

  it("keeps legacy devices understandable when stream state is absent", () => {
    expect(tileStreamStateView(undefined, true, false)).toEqual({
      state: "live",
      label: "Live",
    });
    expect(tileStreamStateView(undefined, false, true)).toEqual({
      state: "error",
      label: "Error",
    });
    expect(tileStreamStateView(undefined, false, false)).toEqual({
      state: "parked",
      label: "Parked",
    });
  });
});

describe("markDeviceFrameLive", () => {
  it("marks only the device that emitted a fresh frame as live", () => {
    const fixture = (udid: string): DeviceInfo => ({
      udid,
      name: udid,
      model: "fixture",
      iosVersion: "fixture",
      connection: "mock",
      status: "ready",
      wdaReady: true,
      tileStreamState: "sampling",
    });
    const devices = [fixture("a"), fixture("b")];

    const next = markDeviceFrameLive(devices, "b");

    expect(next[0]).toBe(devices[0]);
    expect(next[0].tileStreamState).toBe("sampling");
    expect(next[1].tileStreamState).toBe("live");
  });
});
