import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { DeviceDetailsDrawer } from "./DeviceDetailsDrawer";
import { deviceActionCapabilities, deviceAppCandidates, deviceAppSelect } from "../api";

vi.mock("../api", () => ({
  deviceActionCapabilities: vi.fn(),
  deviceAppCandidates: vi.fn(),
  deviceAppSelect: vi.fn(),
  getDeviceMeta: vi.fn(async (udid: string) => ({ udid, notes: "", tags: [] })),
  tiktokCredentialsList: vi.fn(async () => []),
}));

const device = {
  udid: "dual-phone",
  name: "Galaxy",
  model: "SM-G955N",
  platform: "android" as const,
  osVersion: "9",
  connection: "usb" as const,
  status: "ready" as const,
  battery: 80,
  wdaReady: true,
  wdaExpiresAt: null,
  streamUrl: null,
  tileStreamState: "parked" as const,
  lastError: null,
};

beforeEach(() => vi.clearAllMocks());

it("lists both installed TikTok packages and persists the explicit per-device choice", async () => {
  vi.mocked(deviceAppCandidates).mockResolvedValue({
    udid: device.udid,
    appKey: "tiktok",
    installedPackages: ["com.ss.android.ugc.trill", "com.zhiliaoapp.musically"],
    selectedPackage: null,
    suggestedPackage: "com.zhiliaoapp.musically",
    revision: 0,
    selectionValid: false,
    reason: "Thiết bị có nhiều bản TikTok; hãy chọn ứng dụng cần dùng",
  });
  vi.mocked(deviceAppSelect).mockResolvedValue({
    udid: device.udid,
    appKey: "tiktok",
    installedPackages: ["com.ss.android.ugc.trill", "com.zhiliaoapp.musically"],
    selectedPackage: "com.zhiliaoapp.musically",
    suggestedPackage: null,
    revision: 1,
    selectionValid: true,
    reason: null,
  });
  render(<DeviceDetailsDrawer device={device} machineLabel="Máy 24" currentOwner={null} ownerReadFailed={false} onClose={vi.fn()}/>);

  fireEvent.click(screen.getByRole("button", { name: "Chọn ứng dụng TikTok" }));
  expect(await screen.findByText("TikTok Global")).toBeInTheDocument();
  expect(screen.getByText("TikTok Trill")).toBeInTheDocument();
  fireEvent.click(screen.getByLabelText(/TikTok Global/));
  await waitFor(() => expect(deviceAppSelect).toHaveBeenCalledWith(
    "dual-phone", "com.zhiliaoapp.musically", 0,
  ));
  expect(screen.getByLabelText(/TikTok Global/)).toBeChecked();
});

it("shows a build outside the fleet once, as a refusal, instead of per-action reasons", async () => {
  const reason = "TikTok trên máy đang là com.zhiliaoapp.musically 46.4.3 (en); đội máy chỉ chạy TikTok Global 45.7.3 tiếng Anh. Chưa thực hiện thao tác nào. Hạ về 45.7.3 trong Trung tâm ứng dụng rồi kiểm tra lại.";
  vi.mocked(deviceActionCapabilities).mockResolvedValue({
    udid: device.udid,
    package: "com.zhiliaoapp.musically",
    version: "46.4.3",
    locale: "en",
    actions: ["feed", "like", "photo"].map((action) => ({ action, state: "buildNotAllowed" as const, reason })),
  });
  render(<DeviceDetailsDrawer device={device} machineLabel="Máy 24" currentOwner={null} ownerReadFailed={false} onClose={vi.fn()}/>);

  fireEvent.click(screen.getByRole("button", { name: "Kiểm tra khả năng" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(reason);
  expect(screen.getAllByText("Bản TikTok không được phép")).toHaveLength(3);
  expect(screen.getAllByText(reason)).toHaveLength(1);
});

it("does not allow changing the selected app while another owner holds the phone", async () => {
  render(<DeviceDetailsDrawer device={device} machineLabel="Máy 24" currentOwner="interaction" ownerReadFailed={false} onClose={vi.fn()}/>);
  expect(screen.getByRole("button", { name: "Chọn ứng dụng TikTok" })).toBeDisabled();
  expect(deviceAppCandidates).not.toHaveBeenCalled();
});
