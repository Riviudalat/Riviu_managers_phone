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
