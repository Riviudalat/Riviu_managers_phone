import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { deviceThreadsBuild } from "../api";
import type { DeviceInfo } from "../types";
import { DeviceDetailsDrawer } from "./DeviceDetailsDrawer";

vi.mock("../api", () => ({
  deviceThreadsBuild: vi.fn(),
  deviceActionCapabilities: vi.fn(),
}));

const readBuild = vi.mocked(deviceThreadsBuild);
const device: DeviceInfo = {
  udid: "android-1",
  name: "Redmi",
  model: "Note",
  platform: "android",
  osVersion: "15",
  connection: "usb",
  status: "ready",
  wdaReady: false,
};

beforeEach(() => readBuild.mockReset());
afterEach(cleanup);

describe("DeviceDetailsDrawer Threads version", () => {
  const open = () => render(<DeviceDetailsDrawer device={device} machineLabel="Máy 1"
    currentOwner={null} ownerReadFailed={false} onClose={() => {}} />);

  it("reads and shows the installed version for this device", async () => {
    readBuild.mockResolvedValue({ packageName: "com.instagram.barcelona", version: "410.0.0", locale: "vi-VN" });
    open();
    await userEvent.click(screen.getByRole("button", { name: "Kiểm tra phiên bản" }));

    await waitFor(() => expect(readBuild).toHaveBeenCalledWith("android-1"));
    expect(await screen.findByText("410.0.0")).toBeVisible();
    expect(screen.getByText("com.instagram.barcelona")).toBeVisible();
  });
});
