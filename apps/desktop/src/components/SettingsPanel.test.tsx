import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SettingsPanel } from "./SettingsPanel";
import { updateCheck, updateInstall } from "../api";

vi.mock("../api", () => ({
  agentGetSettings: vi.fn(async () => ({
    settings: { autoRepair: false },
    tokenConfigured: true,
    activeArtifactId: "riviu-agent",
    activeArtifactVersion: "1.0.0",
  })),
  agentListStatuses: vi.fn(async () => []),
  agentPreflight: vi.fn(async () => undefined),
  agentRepair: vi.fn(async () => undefined),
  agentSaveSettings: vi.fn(async () => undefined),
  clearAppleId: vi.fn(async () => undefined),
  driverMode: vi.fn(async () => "real"),
  getAppleId: vi.fn(async () => ({ email: "", hasPassword: false })),
  setAppleId: vi.fn(async () => undefined),
  updateCheck: vi.fn(),
  updateInstall: vi.fn(async () => undefined),
}));

const checkMock = vi.mocked(updateCheck);
const installMock = vi.mocked(updateInstall);

beforeEach(() => {
  checkMock.mockReset();
  installMock.mockReset();
  installMock.mockResolvedValue(undefined);
});

afterEach(cleanup);

function installButton() {
  return screen.getByRole("button", { name: "Tải và cài đặt" });
}

describe("SettingsPanel update section", () => {
  it("does not check for an update on mount", async () => {
    // A farm machine is frequently offline and nobody asked it to phone home. This is the
    // wiring half of that promise; the backend never checks on its own either.
    render(<SettingsPanel devices={[]} />);
    await waitFor(() => expect(screen.getByText("Chưa kiểm bản mới")).toBeTruthy());

    expect(checkMock).not.toHaveBeenCalled();
  });

  it("keeps the install button disabled until a check finds a version", async () => {
    render(<SettingsPanel devices={[]} />);
    await waitFor(() => expect(screen.getByText("Chưa kiểm bản mới")).toBeTruthy());

    expect(installButton()).toBeDisabled();
  });

  it("refuses the install while a session is running, and names it", async () => {
    // The load-bearing test of this section: the mapper deciding canInstall is worthless if
    // the JSX ignores it, and an enabled button here would let an operator swap the binary
    // out from under sixteen phones.
    checkMock.mockResolvedValue({
      available: true,
      version: "0.1.2",
      current: "0.1.1",
      busyReason: "2 phiên Nuôi TT đang chạy — dừng hết trước khi cập nhật",
    });
    render(<SettingsPanel devices={[]} />);
    await userEvent.click(screen.getByRole("button", { name: "Kiểm bản mới" }));

    await waitFor(() => expect(screen.getByText(/Nuôi TT đang chạy/)).toBeTruthy());
    expect(installButton()).toBeDisabled();
    expect(installMock).not.toHaveBeenCalled();
  });

  it("offers the install once a version exists and the fleet is idle", async () => {
    checkMock.mockResolvedValue({
      available: true,
      version: "0.1.2",
      current: "0.1.1",
      busyReason: null,
    });
    render(<SettingsPanel devices={[]} />);
    await userEvent.click(screen.getByRole("button", { name: "Kiểm bản mới" }));

    await waitFor(() => expect(installButton()).not.toBeDisabled());
    await userEvent.click(installButton());

    expect(installMock).toHaveBeenCalledTimes(1);
  });

  it("shows a failed check instead of leaving the panel looking current", async () => {
    checkMock.mockRejectedValue(new Error("dns error"));
    render(<SettingsPanel devices={[]} />);
    await userEvent.click(screen.getByRole("button", { name: "Kiểm bản mới" }));

    await waitFor(() => expect(screen.getByText("Không kiểm được bản mới")).toBeTruthy());
    expect(installButton()).toBeDisabled();
  });

  it("reports an install that failed after the fleet was already released", async () => {
    // Only reachable off Windows, where the process survives a failed install. The message
    // has to say the phones are already let go, or an operator retries into a dead app.
    checkMock.mockResolvedValue({
      available: true,
      version: "0.1.2",
      current: "0.1.1",
      busyReason: null,
    });
    installMock.mockRejectedValue(new Error("cài bản mới thất bại sau khi đã dừng phiên"));
    render(<SettingsPanel devices={[]} />);
    await userEvent.click(screen.getByRole("button", { name: "Kiểm bản mới" }));
    await waitFor(() => expect(installButton()).not.toBeDisabled());

    await userEvent.click(installButton());

    await waitFor(() => expect(screen.getByText(/đã dừng phiên/)).toBeTruthy());
  });
});
