import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { DeviceSettingsPage } from "./DeviceSettingsPage";
import type { DeviceBaselineConfig, DeviceInfo } from "../types";

const api = vi.hoisted(() => ({
  deviceBaselineGetConfig: vi.fn(),
  deviceBaselineSaveConfig: vi.fn(),
  deviceBaselineRead: vi.fn(),
  deviceBaselineApply: vi.fn(),
  listGroups: vi.fn(),
}));
vi.mock("../api", () => api);
const confirm = vi.hoisted(() => vi.fn());
vi.mock("../confirmStore", () => ({ requestConfirm: confirm }));

const phone = (udid: string, name: string, platform: "android" | "ios") =>
  ({ udid, name, platform, model: "", osVersion: "", connection: "usb", status: "ready" }) as unknown as DeviceInfo;

const devices = [
  phone("unlocked", "Máy 01", "android"),
  phone("busy", "Máy 02", "android"),
  phone("pin", "Máy 03", "android"),
  phone("iphone", "iPhone 04", "ios"),
];

const defaults: DeviceBaselineConfig = { settings: ["lockScreenDisabled", "autoRotateOff"], autoApplyOnConnect: true };

function row(name: string) {
  return screen.getByRole("rowheader", { name }).closest("tr") as HTMLElement;
}

beforeEach(() => {
  for (const mock of Object.values(api)) mock.mockReset();
  confirm.mockReset();
  confirm.mockResolvedValue(true);
  api.listGroups.mockResolvedValue([]);
  api.deviceBaselineGetConfig.mockResolvedValue(defaults);
});

describe("DeviceSettingsPage", () => {
  it("shows a failed read as unknown, never as matching the baseline", async () => {
    api.deviceBaselineRead.mockImplementation(async (udid: string) =>
      udid === "unlocked"
        ? {
            udid,
            settings: [
              { setting: "lockScreenDisabled", status: "ok", observed: "mã khóa=không lockscreen.disabled=true" },
              { setting: "autoRotateOff", status: "drift", observed: "accelerometer_rotation=1 user_rotation=0" },
            ],
          }
        : { udid, settings: [], error: "Không đọc được cài đặt máy: adb shell timed out after 20s" });
    render(<DeviceSettingsPage devices={devices} />);
    await screen.findByText("Tắt khóa màn hình", { selector: "strong" });

    await userEvent.click(screen.getByRole("button", { name: "Kiểm tra" }));

    await waitFor(() => expect(within(row("Máy 01")).getByText("Đúng chuẩn")).toBeVisible());
    expect(within(row("Máy 01")).getByText("Chưa đúng")).toBeVisible();
    const failed = row("Máy 02");
    expect(within(failed).queryByText("Đúng chuẩn")).toBeNull();
    expect(within(failed).getAllByText("Chưa rõ")).toHaveLength(5);
    expect(within(failed).getByText(/adb shell timed out/)).toBeVisible();
    // iOS is never probed.
    expect(api.deviceBaselineRead).not.toHaveBeenCalledWith("iphone");
    expect(within(row("iPhone 04")).getByText("Chưa hỗ trợ")).toBeVisible();
  });

  it("applies the saved baseline per phone and shows each typed outcome", async () => {
    api.deviceBaselineApply.mockImplementation(async (udid: string) => {
      if (udid === "busy") return { udid, outcome: "refusedBusy", items: [], detail: "Máy đang bận (Script); không chiếm quyền, chưa thay đổi gì" };
      if (udid === "pin") {
        return {
          udid,
          outcome: "needsManual",
          items: [
            { setting: "lockScreenDisabled", outcome: "needsManual", observed: "mã khóa=có", detail: "Cần mở khóa bằng tay: máy có mã PIN/mật khẩu/hình vẽ." },
            { setting: "autoRotateOff", outcome: "applied", observed: "accelerometer_rotation=0 user_rotation=0" },
          ],
        };
      }
      return { udid, outcome: "applied", items: [] };
    });
    render(<DeviceSettingsPage devices={devices} />);
    const applyButton = await screen.findByRole("button", { name: "Áp dụng cho 3 máy" });

    await userEvent.click(applyButton);

    await waitFor(() => expect(within(row("Máy 01")).getByText("Đã áp dụng")).toBeVisible());
    expect(within(row("Máy 02")).getByText("Máy đang bận")).toBeVisible();
    expect(within(row("Máy 03")).getByText("Cần làm tay")).toBeVisible();
    expect(within(row("Máy 03")).getByText(/mã PIN/)).toBeVisible();
    expect(api.deviceBaselineApply).toHaveBeenCalledTimes(3);
    expect(api.deviceBaselineApply).toHaveBeenCalledWith("unlocked", ["lockScreenDisabled", "autoRotateOff"]);
    expect(api.deviceBaselineApply).not.toHaveBeenCalledWith("iphone", expect.anything());
  });

  it("does nothing when the operator cancels the confirmation", async () => {
    confirm.mockResolvedValue(false);
    render(<DeviceSettingsPage devices={devices} />);
    await userEvent.click(await screen.findByRole("button", { name: "Áp dụng cho 3 máy" }));
    expect(api.deviceBaselineApply).not.toHaveBeenCalled();
  });

  it("persists the auto-apply option and blocks apply until the baseline is saved", async () => {
    api.deviceBaselineSaveConfig.mockImplementation(async (config: DeviceBaselineConfig) => config);
    render(<DeviceSettingsPage devices={devices} />);
    const autoApply = await screen.findByRole("checkbox", { name: /Tự áp dụng khi máy kết nối/ });
    expect(autoApply).toBeChecked();

    await userEvent.click(autoApply);
    await userEvent.click(screen.getByRole("checkbox", { name: /Tắt hiệu ứng chuyển động/ }));

    expect(screen.getByText("Chưa lưu")).toBeVisible();
    expect(screen.getByRole("button", { name: "Áp dụng cho 3 máy" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Lưu chuẩn" }));
    await waitFor(() => expect(screen.getByText("Đã lưu")).toBeVisible());
    expect(api.deviceBaselineSaveConfig).toHaveBeenCalledWith({
      settings: ["lockScreenDisabled", "autoRotateOff", "animationsOff"],
      autoApplyOnConnect: false,
    });
    expect(screen.getByRole("button", { name: "Áp dụng cho 3 máy" })).toBeEnabled();
  });
});
