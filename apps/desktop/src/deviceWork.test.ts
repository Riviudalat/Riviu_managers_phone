import { describe, expect, it } from "vitest";

import {
  deviceMatchesFleetFilter,
  deviceOperationalView,
  deviceWorkOwnerLabel,
} from "./deviceWork";
import type { DeviceInfo } from "./types";

const readyDevice: DeviceInfo = {
  udid: "serial-should-stay-private",
  name: "SM-G955F",
  model: "SM-G955F",
  platform: "android",
  osVersion: "9",
  connection: "usb",
  status: "ready",
  wdaReady: false,
};

describe("device work owner presentation", () => {
  it("labels every known owner for the operator", () => {
    expect(deviceWorkOwnerLabel("nurture")).toBe("Nuôi TikTok");
    expect(deviceWorkOwnerLabel("interaction")).toBe("Tương tác");
    expect(deviceWorkOwnerLabel("script")).toBe("Flow");
  });

  it("does not render a future wire value as blank or idle", () => {
    expect(deviceWorkOwnerLabel("futureOwner")).toBe("Tác vụ chưa nhận diện");
  });
});

describe("device operational status", () => {
  it("uses the current owner as Busy and names that owner separately", () => {
    expect(deviceOperationalView(readyDevice, "interaction")).toEqual({
      kind: "busy",
      label: "Bận",
      ownerLabel: "Tương tác",
      tone: "warn",
    });
  });

  it.each([
    [{ status: "ready", wdaReady: false }, "ready", "Sẵn sàng", "ok"],
    [{ status: "connected", wdaReady: true }, "ready", "Sẵn sàng", "ok"],
    [{ status: "busy", wdaReady: false }, "busy", "Bận", "warn"],
    [{ status: "error", wdaReady: false }, "warning", "Cần xem", "warn"],
    [{ status: "disconnected", wdaReady: false }, "offline", "Ngoại tuyến", "info"],
  ] as const)("maps %o to %s", (overrides, kind, label, tone) => {
    expect(deviceOperationalView({ ...readyDevice, ...overrides }, null)).toEqual({
      kind,
      label,
      ownerLabel: null,
      tone,
    });
  });

  it("keeps an offline phone offline even if a stale work owner remains", () => {
    expect(
      deviceOperationalView(
        { ...readyDevice, status: "disconnected", wdaReady: false },
        "nurture",
      ),
    ).toEqual({
      kind: "offline",
      label: "Ngoại tuyến",
      ownerLabel: "Nuôi TikTok",
      tone: "info",
    });
  });

  it.each([
    ["loading", "Đang đọc tác vụ"],
    ["error", "Chưa đọc được tác vụ"],
  ] as const)(
    "fails closed while the work-owner projection is %s",
    (ownerReadState, label) => {
      expect(deviceOperationalView(readyDevice, null, ownerReadState)).toEqual({
        kind: "warning",
        label,
        ownerLabel: null,
        tone: "warn",
      });
    },
  );
});

describe("device fleet filters", () => {
  it("matches the visible machine number and alias without searching hidden identifiers", () => {
    expect(deviceMatchesFleetFilter(readyDevice, null, 12, "Kệ Trên", "may 12", "all")).toBe(
      true,
    );
    expect(deviceMatchesFleetFilter(readyDevice, null, 12, "Kệ Trên", "ke tren", "all")).toBe(
      true,
    );
    expect(
      deviceMatchesFleetFilter(
        readyDevice,
        null,
        12,
        "Kệ Trên",
        readyDevice.udid,
        "all",
      ),
    ).toBe(false);
  });

  it("applies operational status after the owner is considered", () => {
    expect(deviceMatchesFleetFilter(readyDevice, "nurture", 1, "Máy A", "", "busy")).toBe(
      true,
    );
    expect(deviceMatchesFleetFilter(readyDevice, "nurture", 1, "Máy A", "", "ready")).toBe(
      false,
    );
  });
});
