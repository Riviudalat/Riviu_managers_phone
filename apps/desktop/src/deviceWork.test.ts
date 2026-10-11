import { describe, expect, it } from "vitest";

import {
  deviceMatchesFleetFilter,
  deviceOperationalView,
  deviceStatusText,
  deviceWorkOwnerLabel,
  helperRecoveryTargets,
} from "./deviceWork";
import type { AgentStatus, DeviceInfo, DeviceActivity } from "./types";

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
    expect(deviceWorkOwnerLabel("script")).toBe("Tác vụ tự động");
  });

  it("does not render a future wire value as blank or idle", () => {
    expect(deviceWorkOwnerLabel("futureOwner")).toBe("Tác vụ chưa nhận diện");
  });
});

describe("device operational status", () => {
  it("uses the current owner as Đang chạy and names that owner as the job", () => {
    const view = deviceOperationalView(readyDevice, "interaction");
    expect(view).toMatchObject({ kind: "busy", label: "Đang chạy", detail: "Tương tác" });
    expect(view.axes.work).toMatchObject({ state: "running", label: "Đang chạy", detail: "Tương tác" });
    expect(deviceStatusText(view)).toBe("Đang chạy: Tương tác");
  });

  // No cached control proof is passed here, so an Android row without an owner is "Chưa rõ":
  // a roster `ready` or `wdaReady` never stands in for the helper proof.
  it.each([
    [{ status: "ready", wdaReady: false }, "unknown", "Chưa rõ"],
    [{ status: "connected", wdaReady: true }, "unknown", "Chưa rõ"],
    [{ status: "busy", wdaReady: false }, "busy", "Đang chạy"],
    [{ status: "error", wdaReady: false }, "warning", "Cần xử lý"],
    [{ status: "disconnected", wdaReady: false }, "offline", "Mất kết nối"],
  ] as const)("maps %o to %s", (overrides, kind, label) => {
    expect(deviceOperationalView({ ...readyDevice, ...overrides }, null)).toMatchObject({ kind, label });
  });

  it("uses a publish activity for the shared script owner without leaking it after release", () => {
    const activity: DeviceActivity = { operationId: "publish:live", kind: "publish", label: "Đăng bài", step: "Đang chuẩn bị ảnh", state: "running", updatedAt: null };
    expect(deviceOperationalView(readyDevice, "script", "known", undefined, activity)).toMatchObject({
      kind: "busy", label: "Đang chạy", detail: "Đăng bài", step: "Đang chuẩn bị ảnh",
    });
    expect(deviceOperationalView(readyDevice, null, "known", undefined, activity).step).toBeUndefined();
    expect(deviceOperationalView(readyDevice, "script", "known", undefined, { ...activity, state: "uncertain", step: "Chưa xác nhận kết quả" }).step).toBe("Chưa xác nhận kết quả");
    expect(deviceOperationalView(readyDevice, "script", "known", undefined, { ...activity, state: "succeeded" }).detail).toBe("Tác vụ tự động");
  });

  it("keeps an offline phone offline even if a stale work owner remains", () => {
    expect(
      deviceOperationalView(
        { ...readyDevice, status: "disconnected", wdaReady: false },
        "nurture",
      ),
    ).toMatchObject({
      kind: "offline",
      label: "Mất kết nối",
      reason: "Máy không còn trả lời qua USB/Wi-Fi. Nuôi TikTok vẫn đang giữ máy.",
    });
  });

  it("requires helper attachment proof and applies it to the same fleet filter", () => {
    const agent: AgentStatus = {
      udid: readyDevice.udid, state: "ready", artifactId: "helper", artifactVersion: "1",
      bundleId: "com.riviu.helper", protocolVersion: 1, features: ["helperReady"],
      installedVersion: "1", installedBuild: null, authReady: true, mjpegReady: false,
      sessionReady: true, message: null,
    };
    expect(deviceOperationalView(readyDevice, null, "known", agent).kind).toBe("ready");
    expect(deviceMatchesFleetFilter(readyDevice, null, 1, "Máy A", "", "ready", "known", agent)).toBe(true);
    expect(deviceOperationalView(readyDevice, null, "known", { ...agent, features: [] }).kind).toBe("unchecked");
    expect(deviceOperationalView(readyDevice, null, "known", { ...agent, sessionReady: false }).kind).toBe("unchecked");
    expect(deviceOperationalView({ ...readyDevice, platform: "ios" }, null).kind).toBe("ready");
  });

  it("offers helper recovery on the tile for the roster's latched setup failure", () => {
    const latched: AgentStatus = {
      udid: readyDevice.udid, state: "repairRequired", artifactId: "uiautomator", artifactVersion: "1",
      bundleId: "io.appium.uiautomator2.server", protocolVersion: 1, features: [],
      installedVersion: "1", installedBuild: null, authReady: false, mjpegReady: true, sessionReady: false,
      message: "Riviu Helper: runtime qualification failed; recovery required (no action replay); stage=runtime_acquisition; category=recovery_required; Cần đối soát phiên helper đang giữ; chưa nhận phiên thay thế.",
    };
    expect(deviceOperationalView(readyDevice, null, "known", latched)).toMatchObject({
      kind: "warning", label: "Cần xử lý", detail: "Cần khôi phục helper", helperRecovery: true,
    });
  });
});

describe("toolbar helper recovery scope", () => {
  const status = (udid: string, message: string | null): AgentStatus => ({
    udid, state: message ? "repairRequired" : "ready", artifactId: "uiautomator", artifactVersion: "1",
    bundleId: "io.appium.uiautomator2.server", protocolVersion: 1, features: message ? [] : ["helperReady"],
    installedVersion: "1", installedBuild: null, authReady: !message, mjpegReady: true, sessionReady: !message, message,
  });
  const latch = "Riviu Helper: runtime qualification failed; recovery required (no action replay)";

  it("targets only connected Android phones that report a helper recovery need", () => {
    const devices: DeviceInfo[] = [
      { ...readyDevice, udid: "healthy" },
      { ...readyDevice, udid: "latched", status: "error" },
      { ...readyDevice, udid: "roster-only", status: "error", lastError: latch },
      { ...readyDevice, udid: "offline", status: "disconnected" },
      { ...readyDevice, udid: "iphone", platform: "ios" },
      { ...readyDevice, udid: "other-failure", status: "error" },
    ];
    const agents = new Map([
      ["healthy", status("healthy", null)],
      ["latched", status("latched", `${latch}; stage=runtime_acquisition; category=shell_root; adb shell đang chạy quyền root (uid 0)`)],
      ["offline", status("offline", latch)],
      ["iphone", status("iphone", "HelperRecoveryRequired: owner_conflict")],
      ["other-failure", status("other-failure", "the agent on other-failure did not answer /status within 10 seconds")],
    ]);
    expect(helperRecoveryTargets(devices, udid => agents.get(udid))).toEqual(["latched", "roster-only"]);
  });

  it.each([
    ["loading", "đang đọc tác vụ"],
    ["error", "chưa đọc được tác vụ"],
  ] as const)(
    "fails closed while the work-owner projection is %s",
    (ownerReadState, detail) => {
      expect(deviceOperationalView(readyDevice, null, ownerReadState)).toMatchObject({
        kind: "unknown",
        label: "Chưa rõ",
        detail,
        axes: { work: { state: "unknown" } },
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
