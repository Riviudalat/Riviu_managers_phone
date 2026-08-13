import { describe, expect, it } from "vitest";
import {
  deviceModelOsLabel,
  deviceOsLabel,
  markDeviceFrameLive,
  tileStreamStateView,
  type DeviceInfo,
} from "../types";

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

describe("deviceOsLabel", () => {
  it("names the OS the device actually runs", () => {
    // The bug this replaces: the focus dock printed a hardcoded "iOS" beside a
    // version field that carried the Android release, so a Redmi read "iOS 15".
    expect(deviceOsLabel({ platform: "android", osVersion: "15" })).toBe("Android 15");
    expect(deviceOsLabel({ platform: "ios", osVersion: "16.7.15" })).toBe("iOS 16.7.15");
  });

  it("never invents a platform name it does not recognise", () => {
    // A future backend, or a stale payload. Printing the bare version is wrong-ish;
    // printing "iOS" would be actively misleading.
    const unknown = { platform: "windowsphone", osVersion: "8.1" } as unknown as DeviceInfo;
    expect(deviceOsLabel(unknown)).toBe("8.1");
  });

  it("drops the separator rather than trailing a dangling middot", () => {
    expect(deviceModelOsLabel({ model: "Redmi Note 12", platform: "android", osVersion: "15" })).toBe(
      "Redmi Note 12 · Android 15",
    );
    // Annotated rather than inferred: a bare object literal widens `platform` to
    // `string`, which `tsc -b` rejects while `vitest` happily runs it — the exact gap
    // that let this file drift.
    const noVersion: Pick<DeviceInfo, "model" | "platform" | "osVersion"> = {
      model: "Redmi Note 12",
      platform: "android",
      osVersion: "",
    };
    expect(deviceModelOsLabel(noVersion)).toBe("Redmi Note 12 · Android");
  });
});

describe("markDeviceFrameLive", () => {
  it("marks only the device that emitted a fresh frame as live", () => {
    const fixture = (udid: string): DeviceInfo => ({
      udid,
      name: udid,
      model: "fixture",
      platform: "ios",
      osVersion: "fixture",
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
