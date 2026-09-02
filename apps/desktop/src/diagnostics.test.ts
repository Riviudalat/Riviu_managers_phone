import { describe, expect, it, vi } from "vitest";

import {
  createHealthLimiter,
  loadFleetHealth,
  normalizeDeviceHealth,
  withHealthDeadline,
  type HealthStatus,
} from "./diagnostics";
import type { DeviceHealthReport, DeviceInfo } from "./types";

const device: DeviceInfo = {
  udid: "redmi-1",
  name: "Redmi 12C",
  model: "23021RAAEG",
  platform: "android",
  osVersion: "15",
  connection: "usb",
  status: "ready",
  wdaReady: false,
};

function report(overrides: Partial<DeviceHealthReport> = {}): DeviceHealthReport {
  return {
    udid: device.udid,
    rosterStatus: "ready",
    agent: { state: "ready" } as DeviceHealthReport["agent"],
    agentReadyNow: true,
    agentFeatures: ["tap", "swipe", "text"],
    agentAuthReady: true,
    adbPath: "C:\\Riviu\\adb.exe",
    adbOrigin: "bản đóng gói trong bộ cài",
    adbVersion: "Android Debug Bridge version 1.0.41",
    helperReachable: true,
    helperInstalled: true,
    root: { hasSu: true, shellIsRoot: true },
    tiktokPackage: "com.zhiliaoapp.musically",
    tiktokVersion: "40.1.3",
    tiktokLocale: "vi-VN",
    geometry: { width: 1080, height: 2220, density: 420, rotation: 0 },
    streamGeneration: 3,
    notes: [],
    ...overrides,
  };
}

describe("normalizeDeviceHealth", () => {
  it("normalizes every read-only backend answer into named checks", () => {
    const checks = normalizeDeviceHealth(device, report());

    expect(checks.map((check) => check.id)).toEqual([
      "roster",
      "transport",
      "adb",
      "agentCache",
      "agentLive",
      "agentCapabilities",
      "helperInstalled",
      "helperReachable",
      "root",
      "geometry",
      "stream",
      "tiktok",
    ]);
    expect(checks.map((check) => check.status)).toEqual<HealthStatus[]>([
      "pass", "pass", "pass", "pass", "pass", "pass", "pass", "pass", "pass", "pass", "pass", "pass",
    ]);
  });

  it("keeps unknown answers distinct from a negative answer", () => {
    const checks = normalizeDeviceHealth(device, report({
      agentReadyNow: null,
      agentFeatures: null,
      agentAuthReady: null,
      adbPath: null,
      helperInstalled: null,
      helperReachable: null,
      root: null,
      tiktokPackage: null,
      geometry: null,
    }));

    expect(checks.find((check) => check.id === "agentLive")).toMatchObject({ status: "unknown" });
    expect(checks.find((check) => check.id === "agentCapabilities")).toMatchObject({ status: "unknown" });
    expect(checks.find((check) => check.id === "helperInstalled")).toMatchObject({ status: "unknown" });
    expect(checks.find((check) => check.id === "helperReachable")).toMatchObject({ status: "unknown" });
    expect(checks.find((check) => check.id === "root")).toMatchObject({ status: "unknown" });
    expect(checks.find((check) => check.id === "tiktok")).toMatchObject({ status: "unknown" });
    expect(checks.find((check) => check.id === "geometry")).toMatchObject({ status: "unknown" });
    expect(checks.find((check) => check.id === "stream")).toMatchObject({ status: "pass" });
    expect(checks.some((check) => /Không|No/i.test(check.summary))).toBe(false);
  });

  it("reports a missing helper and a failing live agent as failures", () => {
    const checks = normalizeDeviceHealth(device, report({
      agentReadyNow: false,
      helperInstalled: false,
      helperReachable: false,
    }));

    expect(checks.find((check) => check.id === "agentLive")).toMatchObject({ status: "fail" });
    expect(checks.find((check) => check.id === "helperInstalled")).toMatchObject({ status: "fail" });
    expect(checks.find((check) => check.id === "helperReachable")).toMatchObject({ status: "warning" });
  });

  it("never promotes an iOS device to Android from contradictory cached evidence", () => {
    const ios: DeviceInfo = { ...device, udid: "iphone-1", platform: "ios" };
    const checks = normalizeDeviceHealth(ios, report({
      udid: ios.udid,
      agentReadyNow: true,
      adbPath: "C:\\stale\\adb.exe",
      root: { hasSu: true, shellIsRoot: true },
      tiktokPackage: "com.zhiliaoapp.musically",
    }));

    for (const id of ["transport", "adb", "agentLive", "agentCapabilities", "helperInstalled", "helperReachable", "root", "geometry", "tiktok"] as const) {
      expect(checks.find((check) => check.id === id)).toMatchObject({ status: "notApplicable" });
    }
  });
});

describe("loadFleetHealth", () => {
  it("settles a row whose IPC promise never answers", async () => {
    await expect(withHealthDeadline(
      () => new Promise<DeviceHealthReport>(() => undefined),
      5,
    )).rejects.toThrow("Chẩn đoán máy không trả lời sau 1 giây");
  });

  it("starts no more than four read-only health calls at once and preserves device order", async () => {
    const devices = Array.from({ length: 9 }, (_, index) => ({
      ...device,
      udid: `redmi-${index + 1}`,
    }));
    let active = 0;
    let highWater = 0;
    const gates = new Map<string, () => void>();
    const read = vi.fn((udid: string) => new Promise<DeviceHealthReport>((resolve) => {
      active += 1;
      highWater = Math.max(highWater, active);
      gates.set(udid, () => {
        active -= 1;
        resolve(report({ udid }));
      });
    }));
    const seen: string[] = [];
    const pending = loadFleetHealth(devices, read, (row) => seen.push(row.device.udid));

    await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(4));
    expect(highWater).toBe(4);
    expect(seen).toEqual([]);

    for (const current of devices) {
      await vi.waitFor(() => expect(gates.get(current.udid)).toBeTypeOf("function"));
      gates.get(current.udid)?.();
    }

    const rows = await pending;
    expect(highWater).toBe(4);
    expect(rows.map((row) => row.device.udid)).toEqual(devices.map((item) => item.udid));
    expect(seen).toEqual(devices.map((item) => item.udid));
  });

  it("retains a row-level failure while the remaining devices continue", async () => {
    const devices = [{ ...device, udid: "one" }, { ...device, udid: "two" }];
    const rows = await loadFleetHealth(devices, async (udid) => {
      if (udid === "one") throw new Error("cable disconnected");
      return report({ udid });
    });

    expect(rows[0]).toMatchObject({
      error: "Không đọc được trạng thái máy. Hãy kiểm lại.",
      errorDetail: "cable disconnected",
    });
    expect(rows[1].report?.udid).toBe("two");
  });

  it("shares one four-slot limiter between initial loading and a row retry", async () => {
    const devices = Array.from({ length: 4 }, (_, index) => ({ ...device, udid: `first-${index}` }));
    const limiter = createHealthLimiter();
    let active = 0;
    let highWater = 0;
    const releases: Array<() => void> = [];
    const read = vi.fn((udid: string) => new Promise<DeviceHealthReport>((resolve) => {
      active += 1;
      highWater = Math.max(highWater, active);
      releases.push(() => { active -= 1; resolve(report({ udid })); });
    }));

    const initial = loadFleetHealth(devices, read, undefined, 4, limiter);
    await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(4));
    const retry = limiter.run(() => read("retry"));
    await Promise.resolve();
    expect(read).toHaveBeenCalledTimes(4);

    releases.shift()?.();
    await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(5));
    expect(highWater).toBe(4);
    while (releases.length) releases.shift()?.();
    await initial;
    await retry;
  });
});
