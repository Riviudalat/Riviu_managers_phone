import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AgentSection } from "./AgentSection";
import type { AgentStatus, DeviceInfo } from "../../types";

const api = vi.hoisted(() => ({
  agentGetSettings: vi.fn(),
  agentListStatuses: vi.fn(),
  agentPreflight: vi.fn(),
  agentRepair: vi.fn(),
  agentSaveSettings: vi.fn(),
}));

vi.mock("../../api", () => api);

const device: DeviceInfo = {
  udid: "android-01",
  name: "Máy 01",
  model: "Pixel",
  platform: "android",
  osVersion: "15",
  connection: "usb",
  status: "ready",
  wdaReady: false,
};

const unknownStatus: AgentStatus = {
  udid: device.udid,
  state: "unknown",
  artifactId: "agent",
  artifactVersion: "1",
  bundleId: "com.riviu.agent",
  protocolVersion: 0,
  features: [],
  installedVersion: null,
  installedBuild: null,
  authReady: false,
  mjpegReady: false,
  sessionReady: false,
  message: null,
};

beforeEach(() => {
  vi.clearAllMocks();
  api.agentGetSettings.mockResolvedValue({
    settings: { autoRepair: true },
    tokenConfigured: true,
    activeArtifactId: "agent",
    activeArtifactVersion: "1",
  });
  api.agentListStatuses.mockResolvedValue([unknownStatus]);
});

describe("AgentSection states", () => {
  it("keeps iOS configuration in a disclosure without warning about Android authentication", async () => {
    api.agentGetSettings.mockResolvedValue({ settings: { autoRepair: false }, tokenConfigured: false,
      activeArtifactId: "ios-agent-unavailable", activeArtifactVersion: "unknown" });
    render(<AgentSection connectedDevices={[device]} connectedUdids={[device.udid]} iosRuntimeIssue="Missing iOS credential" />);
    await waitFor(() => expect(api.agentGetSettings).toHaveBeenCalled());
    expect(screen.queryByText("Chưa cấu hình xác thực iOS")).toBeNull();
    const reason = screen.getByText(/Missing iOS credential/);
    expect(reason).not.toBeVisible();
    fireEvent.click(screen.getByText("Cấu hình Agent iOS", { selector: "summary" }));
    expect(reason).toBeVisible();
    expect(screen.getByText(/không quyết định trạng thái Agent Android/)).toBeVisible();
    expect(api.agentRepair).not.toHaveBeenCalled();
    expect(api.agentPreflight).not.toHaveBeenCalled();
  });

  it("does not report an Android protocol as the iOS Agent protocol", async () => {
    api.agentListStatuses.mockResolvedValue([{ ...unknownStatus, protocolVersion: 1 }]);
    render(<AgentSection connectedDevices={[device]} connectedUdids={[device.udid]} />);
    await waitFor(() => expect(api.agentListStatuses).toHaveBeenCalled());
    expect(screen.getByText("Giao thức").nextElementSibling).toHaveTextContent("Chưa rõ");
  });

  it("shows authentication status explicitly for connected iPhones", async () => {
    const ios = { ...device, udid: "iphone-1", platform: "ios" as const };
    render(<AgentSection connectedDevices={[ios]} connectedUdids={[ios.udid]} />);
    expect(await screen.findByText("Đã cấu hình xác thực iOS")).toBeVisible();
    expect(screen.queryByText(/kho thông tin xác thực Windows/)).toBeNull();
  });
  it("only saves auto-repair after Apply and never invokes a device repair", async () => {
    api.agentSaveSettings.mockImplementation(async (settings) => ({ settings, tokenConfigured: true, activeArtifactId: "agent", activeArtifactVersion: "1" }));
    render(<AgentSection connectedDevices={[device]} connectedUdids={[device.udid]} />);
    const checkbox = screen.getByRole("checkbox", { name: "Tự khôi phục Agent" });
    await waitFor(() => expect(checkbox).toBeChecked());
    fireEvent.click(checkbox);
    expect(api.agentSaveSettings).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Lưu tự khôi phục" }));
    await waitFor(() => expect(api.agentSaveSettings).toHaveBeenCalledExactlyOnceWith({ autoRepair: false }));
    expect(api.agentRepair).not.toHaveBeenCalled();
  });
  it("does not render unknown readiness fields as No", async () => {
    render(<AgentSection connectedDevices={[device]} connectedUdids={[device.udid]} />);

    await waitFor(() => expect(screen.getByText("Chưa kiểm tra")).toBeInTheDocument());
    const row = screen.getByText("Máy 01").closest("[role='row']");
    expect(row).toHaveTextContent("Chưa rõ");
    expect(row).not.toHaveTextContent("No");
  });

  it("uses the fleet number and alias as the primary label, with model and serial in details", async () => {
    render(
      <AgentSection
        connectedDevices={[device]}
        connectedUdids={[device.udid]}
        deviceLabels={new Map([[device.udid, "Máy 2 · Canary"]])}
      />,
    );

    expect(await screen.findByText("Máy 2 · Canary")).toBeVisible();
    const model = screen.getByText("Pixel");
    const serial = screen.getByText("android-01");
    expect(model).not.toBeVisible();
    expect(serial).not.toBeVisible();
    expect(model.closest("details")).not.toHaveAttribute("open");
    expect(serial.closest("details")).not.toHaveAttribute("open");
  });

  it("shows a retryable error when status loading fails", async () => {
    api.agentListStatuses
      .mockRejectedValueOnce(new Error("adb inventory unavailable"))
      .mockResolvedValueOnce([unknownStatus]);
    render(<AgentSection connectedDevices={[device]} connectedUdids={[device.udid]} />);

    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("adb inventory unavailable"));
    fireEvent.click(screen.getByRole("button", { name: "Thử lại trạng thái" }));
    await waitFor(() => expect(api.agentListStatuses).toHaveBeenCalledTimes(2));
  });

  it("keeps each Agent row aligned with the seven accessible column headers", async () => {
    render(<AgentSection connectedDevices={[device]} connectedUdids={[device.udid]} />);

    await waitFor(() => expect(screen.getByText("Chưa kiểm tra")).toBeInTheDocument());
    const table = screen.getByRole("table", { name: "Trạng thái Agent" });
    const headers = Array.from(table.querySelectorAll("[role='columnheader']"));
    const row = screen.getByText("Máy 01").closest("[role='row']");
    const cells = Array.from(row!.querySelectorAll(":scope > [role='cell']"));
    const details = screen.getByText("Chi tiết sẵn sàng").closest("details");

    expect(headers).toHaveLength(7);
    expect(cells).toHaveLength(headers.length);
    expect(cells.at(-1)).toContainElement(details);
    expect(details).not.toHaveAttribute("role");
  });
});
