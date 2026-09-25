import { fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";

import type { DeviceInfo } from "../types";
import { InspectorLauncher } from "./InspectorLauncher";

vi.mock("./DeviceInspector", () => ({
  DeviceInspector: ({ udid, onClose }: { udid: string; onClose: () => void }) => (
    <div role="dialog" aria-label="Inspector thiết bị">
      <span>{udid}</span>
      <button onClick={onClose}>Đóng Inspector</button>
    </div>
  ),
}));

const android: DeviceInfo = {
  udid: "android-1",
  name: "Pixel 8",
  model: "Pixel 8",
  platform: "android",
  osVersion: "14",
  connection: "usb",
  status: "ready",
  wdaReady: false,
};
const labels = new Map([
  ["android-1", "Máy 2 · Pixel 8"],
  ["android-2", "Máy 3 · Galaxy S"],
]);

afterEach(() => vi.clearAllMocks());

it("opens a picker containing only connected Android devices, without opening Inspector first", () => {
  render(<InspectorLauncher
    devices={[android, { ...android, udid: "android-2", name: "Galaxy S" },
      { ...android, udid: "offline", status: "disconnected" },
      { ...android, udid: "ios", platform: "ios" }]}
    deviceLabels={labels}
  />);

  expect(screen.queryByRole("dialog")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Mở Inspector" }));
  const picker = screen.getByRole("dialog", { name: "Chọn máy Android cho Inspector" });
  expect(within(picker).getByRole("button", { name: "Máy 2 · Pixel 8" })).toBeVisible();
  expect(within(picker).getByRole("button", { name: "Máy 3 · Galaxy S" })).toBeVisible();
  expect(within(picker).queryByText("offline")).toBeNull();
  expect(within(picker).queryByText("ios")).toBeNull();
  expect(screen.queryByRole("dialog", { name: "Inspector thiết bị" })).toBeNull();

  fireEvent.click(within(picker).getByRole("button", { name: "Máy 3 · Galaxy S" }));
  expect(screen.queryByRole("dialog", { name: "Chọn máy Android cho Inspector" })).toBeNull();
  expect(within(screen.getByRole("dialog", { name: "Inspector thiết bị" })).getByText("android-2")).toBeVisible();
});

it("shows an empty picker and never mounts Inspector when no Android is connected", () => {
  render(<InspectorLauncher devices={[{ ...android, status: "disconnected" }]} deviceLabels={labels} />);
  fireEvent.click(screen.getByRole("button", { name: "Mở Inspector" }));
  const picker = screen.getByRole("dialog", { name: "Chọn máy Android cho Inspector" });
  expect(within(picker).getByText("Không có máy Android đang kết nối.")).toBeVisible();
  fireEvent.click(within(picker).getByRole("button", { name: "Đóng danh sách máy" }));
  expect(screen.queryByRole("dialog")).toBeNull();
});

it("closes Inspector when the selected device disconnects and does not reopen on reconnect", () => {
  const view = render(<InspectorLauncher devices={[android]} deviceLabels={labels} />);
  fireEvent.click(screen.getByRole("button", { name: "Mở Inspector" }));
  fireEvent.click(screen.getByRole("button", { name: "Máy 2 · Pixel 8" }));
  expect(screen.getByRole("dialog", { name: "Inspector thiết bị" })).toBeVisible();

  view.rerender(<InspectorLauncher devices={[{ ...android, status: "disconnected" }]} deviceLabels={labels} />);
  expect(screen.queryByRole("dialog", { name: "Inspector thiết bị" })).toBeNull();
  view.rerender(<InspectorLauncher devices={[android]} deviceLabels={labels} />);
  expect(screen.queryByRole("dialog", { name: "Inspector thiết bị" })).toBeNull();
});
