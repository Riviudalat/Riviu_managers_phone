import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { FleetDiagnosticsPage } from "./FleetDiagnosticsPage";
import type { DeviceHealthReport, DeviceInfo, DeviceMeta } from "../types";

const readHealth = vi.hoisted(() => vi.fn());
vi.mock("../api", () => ({ deviceHealth: readHealth }));

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
const metas: DeviceMeta[] = [{ udid: "redmi-1", notes: "", tags: [], number: 7, alias: "R12" }];

function report(overrides: Partial<DeviceHealthReport> = {}): DeviceHealthReport {
  return {
    udid: "redmi-1",
    rosterStatus: "ready",
    agent: { state: "ready", message: "Probe answered from cache" } as DeviceHealthReport["agent"],
    agentReadyNow: true,
    agentFeatures: ["tap", "swipe"],
    agentAuthReady: true,
    adbPath: "C:\\Riviu\\adb.exe",
    adbOrigin: "bản đóng gói trong bộ cài",
    adbVersion: "Android Debug Bridge version 1.0.41",
    helperReachable: true,
    helperInstalled: true,
    root: { hasSu: false, shellIsRoot: false },
    tiktokPackage: "com.zhiliaoapp.musically",
    tiktokVersion: "40.1.3",
    tiktokLocale: "vi-VN",
    geometry: { width: 1080, height: 2220, density: 420, rotation: 0 },
    streamGeneration: 2,
    notes: ["Probe answered from cache"],
    ...overrides,
  };
}

beforeEach(() => {
  readHealth.mockReset();
});
afterEach(cleanup);

describe("FleetDiagnosticsPage", () => {
  it("renders loading, then machine number, alias, model and accessible check details", async () => {
    let resolve: (value: DeviceHealthReport) => void = () => undefined;
    readHealth.mockImplementationOnce(() => new Promise<DeviceHealthReport>((done) => { resolve = done; }));
    render(<FleetDiagnosticsPage devices={[device]} metas={metas} />);

    expect(screen.getAllByRole("status")[0]).toHaveTextContent("Đang kiểm tra 1 máy");
    resolve(report());

    const row = await screen.findByRole("row", { name: "Máy 7 · R12 · 23021RAAEG" });
    expect(row).toHaveTextContent("Máy 7 · R12");
    expect(row).toHaveTextContent("23021RAAEG");
    await userEvent.click(screen.getByRole("button", { name: "Xem chi tiết Máy 7 · R12 · 23021RAAEG" }));
    const drawer = screen.getByRole("dialog", { name: "Máy 7 · R12 · 23021RAAEG" });
    const overview = within(drawer).getByRole("list", { name: "Trạng thái kiểm tra Máy 7 · R12 · 23021RAAEG" });
    expect(overview).toHaveTextContent("Agent đã ghi nhận");
    expect(overview).toHaveTextContent("Đạt");
    const evidence = within(drawer).getAllByText("Bằng chứng kỹ thuật");
    await userEvent.click(evidence[0]);
    expect(drawer).toHaveTextContent("Probe answered from cache");
  });

  it("shows an explicit unknown label, never a negative answer, when a probe was not answered", async () => {
    readHealth.mockResolvedValueOnce(report({
      helperInstalled: null,
      helperReachable: null,
      root: null,
      tiktokPackage: null,
    }));
    render(<FleetDiagnosticsPage devices={[device]} metas={metas} />);

    await screen.findByRole("row", { name: "Máy 7 · R12 · 23021RAAEG" });
    await userEvent.click(screen.getByRole("button", { name: "Xem chi tiết Máy 7 · R12 · 23021RAAEG" }));
    await waitFor(() => expect(screen.getAllByText("Chưa hỏi được").length).toBeGreaterThan(0));
    expect(screen.queryByText("No")).toBeNull();
    expect(screen.queryByText("Không")).toBeNull();
  });

  it("shows a per-row error and retries only that read", async () => {
    readHealth.mockRejectedValueOnce(new Error("cable disconnected"));
    render(<FleetDiagnosticsPage devices={[device]} metas={metas} />);

    await waitFor(() => expect(screen.getAllByText("Không đọc được")).toHaveLength(4));
    await userEvent.click(screen.getByRole("button", { name: "Xem chi tiết Máy 7 · R12 · 23021RAAEG" }));
    expect(screen.getByText("Không đọc được trạng thái máy. Hãy kiểm lại.")).toBeVisible();
    expect(screen.getByText("cable disconnected")).not.toBeVisible();
    await userEvent.click(screen.getByText("Chi tiết lỗi"));
    expect(await screen.findByText("cable disconnected")).toBeVisible();
    readHealth.mockResolvedValueOnce(report());
    await userEvent.click(screen.getByRole("button", { name: "Kiểm lại Máy 7 · R12 · 23021RAAEG" }));

    await waitFor(() => expect(readHealth).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("Sẵn sàng trong cache")).toBeVisible();
  });

  it("does not restart an in-flight probe when polling refreshes the same roster", async () => {
    let resolve: (value: DeviceHealthReport) => void = () => undefined;
    readHealth.mockImplementation(() => new Promise<DeviceHealthReport>((done) => { resolve = done; }));
    const { rerender } = render(<FleetDiagnosticsPage devices={[device]} metas={metas} />);
    await waitFor(() => expect(readHealth).toHaveBeenCalledTimes(1));

    rerender(<FleetDiagnosticsPage devices={[{ ...device, status: "connected" }]} metas={[...metas]} />);
    await Promise.resolve();
    expect(readHealth).toHaveBeenCalledTimes(1);

    resolve(report());
    await waitFor(() => expect(screen.getByText("1/1 máy đã có kết quả")).toBeVisible());
  });

  it("drops a row retry response after the roster generation changes", async () => {
    let resolveRetry!: (value: DeviceHealthReport) => void;
    const replacement = { ...device, udid: "redmi-2", name: "Redmi 13C" };
    readHealth
      .mockResolvedValueOnce(report())
      .mockImplementationOnce(() => new Promise<DeviceHealthReport>((resolve) => { resolveRetry = resolve; }))
      .mockResolvedValueOnce({ ...report(), udid: "redmi-2", tiktokVersion: "current-roster" });
    const { rerender } = render(<FleetDiagnosticsPage devices={[device]} metas={metas} />);
    await screen.findByRole("row", { name: "Máy 7 · R12 · 23021RAAEG" });
    await userEvent.click(screen.getByRole("button", { name: "Kiểm lại Máy 7 · R12 · 23021RAAEG" }));
    await waitFor(() => expect(readHealth).toHaveBeenCalledTimes(2));

    rerender(<FleetDiagnosticsPage devices={[replacement]} metas={[]} />);
    await waitFor(() => expect(readHealth).toHaveBeenCalledTimes(3));
    expect(await screen.findByRole("row", { name: /Máy 1 · Redmi 13C/ })).toBeVisible();

    resolveRetry(report({ tiktokVersion: "stale-retry" }));
    await Promise.resolve();
    expect(screen.queryByText("stale-retry")).toBeNull();
    expect(screen.getByRole("row", { name: /Máy 1 · Redmi 13C/ })).toBeVisible();
  });

  it("shows the empty state without starting any device command", () => {
    render(<FleetDiagnosticsPage devices={[]} metas={[]} />);

    expect(screen.getByText("Chưa có điện thoại nào")).toBeVisible();
    expect(readHealth).not.toHaveBeenCalled();
  });

  it("exports the normalized report as JSON", async () => {
    readHealth.mockResolvedValueOnce(report());
    const onExport = vi.fn();
    render(<FleetDiagnosticsPage devices={[device]} metas={metas} onExport={onExport} />);
    await screen.findByRole("row", { name: "Máy 7 · R12 · 23021RAAEG" });

    await userEvent.click(screen.getByRole("button", { name: "Xuất JSON" }));
    expect(onExport).toHaveBeenCalledWith(expect.stringContaining('"rows"'));
    expect(onExport.mock.calls[0][0]).toContain('"status": "pass"');
  });
});
