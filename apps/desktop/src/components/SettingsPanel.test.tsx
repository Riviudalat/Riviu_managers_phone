import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SettingsPanel } from "./SettingsPanel";
import { updateCheck, updateInstall } from "../api";
import type { LocalApiConfig } from "../api";

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
  // A whole-module factory: anything the panel imports but this omits is `undefined` and
  // throws on the first call. Both of these are reached on mount.
  getStreamSettings: vi.fn(async () => ({
    fps: 24,
    gridQuality: "medium",
    focusQuality: "high",
  })),
  setStreamSettings: vi.fn(async (settings: unknown) => settings),
  // Reached on mount too — the loopback API config load.
  localApiGetConfig: vi.fn(async () => ({ enabled: false, port: 22222, token: "" })),
  localApiSetConfig: vi.fn(async (config: unknown) => config),
  localApiStatus: vi.fn(async () => ({ configuredEnabled: false, configuredPort: 22222, running: false, activePort: null, restartRequired: false, lastError: null })),
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

    expect(screen.queryByRole("heading", { name: "Cài đặt" })).toBeNull();
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

  it("reports a surviving macOS install as success instead of an update error", async () => {
    checkMock.mockResolvedValue({
      available: true,
      version: "0.1.2",
      current: "0.1.1",
      busyReason: null,
    });
    installMock.mockResolvedValue(undefined);
    render(<SettingsPanel devices={[]} />);
    await userEvent.click(screen.getByRole("button", { name: "Kiểm bản mới" }));
    await waitFor(() => expect(installButton()).not.toBeDisabled());

    await userEvent.click(installButton());

    expect(await screen.findByText("Đã cài xong — mở lại app để dùng bản mới.")).toHaveTextContent(
      "Đã cài xong — mở lại app để dùng bản mới.",
    );
    expect(screen.queryByText("Không kiểm được bản mới")).toBeNull();
    expect(installButton()).toBeDisabled();
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

describe("stream quality", () => {
  it("sends the whole row, not only the field that changed", async () => {
    // `set_stream_settings` takes a complete `StreamSettings`; posting a partial one would
    // silently reset every field the operator did not touch back to its default.
    const api = await import("../api");
    const save = vi.mocked(api.setStreamSettings);
    save.mockClear();
    render(<SettingsPanel devices={[]} />);
    await waitFor(() => expect(screen.getByLabelText("Chất lượng lưới")).toBeTruthy());

    await userEvent.selectOptions(screen.getByLabelText("Chất lượng lưới"), "extra");
    expect(save).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Áp dụng chất lượng hình" }));

    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    expect(save.mock.calls[0][0]).toEqual({
      fps: 24,
      gridQuality: "extra",
      focusQuality: "high",
    });
  });

  it("shows the value the host stored, not the one that was typed", async () => {
    // Rust clamps the frame rate into the range the fleet actually runs at, and the reply is
    // the clamped row. Rendering the typed value instead would tell the operator a rate no
    // phone here delivers -- the UI-and-encoder disagreement AGENTS.md 9.59 records.
    const api = await import("../api");
    const save = vi.mocked(api.setStreamSettings);
    save.mockClear();
    save.mockResolvedValueOnce({
      fps: 30,
      gridQuality: "medium",
      focusQuality: "high",
    });
    render(<SettingsPanel devices={[]} />);
    await waitFor(() => expect(screen.getByLabelText("FPS overlay")).toBeTruthy());

    const fps = screen.getByLabelText("FPS overlay");
    await userEvent.clear(fps);
    await userEvent.type(fps, "15");
    expect(save).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Áp dụng chất lượng hình" }));

    await waitFor(() => expect((fps as HTMLInputElement).value).toBe("30"));
  });

  it("says the number is the overlay's, because a tile will not run at it", async () => {
    // The field is labelled "FPS overlay" and not "FPS" for a reason: tiles are capped
    // below it in `ViewPreset::Tile::max_fps`, so a bare "FPS" would promise the grid a
    // rate it never runs at -- the same UI-and-encoder disagreement the test above guards
    // from the other side. The ceiling itself is checked against Rust by
    // `the_settings_hint_names_the_same_tile_ceiling_this_file_enforces`.
    render(<SettingsPanel devices={[]} />);
    await waitFor(() => expect(screen.getByLabelText("FPS overlay")).toBeTruthy());
    expect(screen.queryByLabelText("FPS")).toBeNull();
    expect(screen.getByText(/Tile trong lưới bị chặn ở 10 hình\/giây/)).toBeTruthy();
    expect(screen.getByRole("group", { name: "Phạm vi chất lượng hình" })).toBeTruthy();
    expect(screen.queryByText(/135% một nhân CPU/)).toBeNull();
  });

  it("keeps consequences visible and implementation detail in disclosures", async () => {
    render(<SettingsPanel devices={[]} />);
    await waitFor(() => expect(screen.getByLabelText("Chất lượng lưới")).toBeTruthy());

    for (const name of [
      "Phạm vi chất lượng hình",
      "Cách đồng bộ nhóm",
      "Điều kiện kết nối Wi-Fi",
      "Phạm vi API cục bộ",
      "Chi tiết kết nối thiết bị",
    ]) {
      expect(screen.getByRole("group", { name })).not.toHaveAttribute("open");
    }
    expect(screen.queryByText("Desktop bridge")).toBeNull();
    expect(screen.queryByText("Legacy stock agent")).toBeNull();
    expect(screen.queryByText(/Mock chỉ dùng khi phát triển/)).toBeNull();
  });
});

describe("independent settings drafts", () => {
  it("groups settings under keyboard-accessible anchors without unmounting draft regions", async () => {
    render(<SettingsPanel devices={[]} />);
    const navigation = screen.getByRole("navigation", { name: "Nhóm cài đặt" });
    for (const label of ["Hình ảnh và điều khiển", "Kết nối và API", "Bảo trì"]) {
      const link = within(navigation).getByRole("link", { name: label });
      expect(globalThis.document.querySelector(link.getAttribute("href")!)).toHaveAttribute("aria-labelledby");
    }
    expect(await screen.findByLabelText("FPS overlay")).toBeVisible();
    expect(await screen.findByLabelText("Cổng")).toBeVisible();
  });
  it("keeps valid multi-digit FPS editable and only sends after Apply", async () => {
    const save = vi.mocked((await import("../api")).setStreamSettings);
    save.mockClear();
    render(<SettingsPanel devices={[]} />);
    const fps = await screen.findByLabelText("FPS overlay");
    await userEvent.clear(fps);
    await userEvent.type(fps, "15");
    expect(fps).toHaveValue(15);
    expect(save).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Áp dụng chất lượng hình" }));
    expect(save).toHaveBeenCalledExactlyOnceWith({ fps: 15, gridQuality: "medium", focusQuality: "high" });
  });

  it("keeps newer FPS edits when an older save returns", async () => {
    const api = await import("../api");
    let complete!: (settings: { fps: number; gridQuality: "medium"; focusQuality: "high" }) => void;
    vi.mocked(api.setStreamSettings).mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }));
    render(<SettingsPanel devices={[]} />);
    const fps = await screen.findByLabelText("FPS overlay");
    fireEvent.change(fps, { target: { value: "15" } });
    await userEvent.click(screen.getByRole("button", { name: "Áp dụng chất lượng hình" }));
    fireEvent.change(fps, { target: { value: "20" } });
    await act(async () => complete({ fps: 15, gridQuality: "medium", focusQuality: "high" }));
    expect(fps).toHaveValue(20);
    expect(screen.getByRole("button", { name: "Áp dụng chất lượng hình" })).toBeEnabled();
  });

  it("rejects incomplete FPS without a request", async () => {
    const save = vi.mocked((await import("../api")).setStreamSettings);
    save.mockClear();
    render(<SettingsPanel devices={[]} />);
    const fps = await screen.findByLabelText("FPS overlay");
    await userEvent.clear(fps);
    await userEvent.click(screen.getByRole("button", { name: "Áp dụng chất lượng hình" }));
    expect(save).not.toHaveBeenCalled();
    expect(screen.getByText("FPS phải là số nguyên từ 5 đến 30.")).toBeVisible();
  });

  it("never reports an active API as stopped after saving disabled configuration", async () => {
    const api = await import("../api");
    vi.mocked(api.localApiGetConfig).mockResolvedValueOnce({ enabled: true, port: 22222, token: "fixture-token" });
    vi.mocked(api.localApiStatus).mockResolvedValue({ configuredEnabled: false, configuredPort: 22222, running: true, activePort: 22222, restartRequired: true, lastError: null });
    render(<SettingsPanel devices={[]} />);
    const enabled = await screen.findByRole("checkbox", { name: "Bật API cục bộ" });
    await userEvent.click(enabled);
    await userEvent.click(screen.getByRole("button", { name: "Lưu API cục bộ" }));
    expect(await screen.findByText(/Đang chạy tại 127.0.0.1:22222/)).toHaveTextContent("Cần khởi động lại");
    expect(screen.queryByText(/API đang tắt/)).not.toBeInTheDocument();
  });

  it("keeps newer API port edits while the previous snapshot saves", async () => {
    const api = await import("../api");
    let complete!: (config: LocalApiConfig) => void;
    vi.mocked(api.localApiSetConfig).mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }));
    render(<SettingsPanel devices={[]} />);
    const region = screen.getByRole("region", { name: "API tự động hoá cục bộ" });
    const port = await within(region).findByLabelText("Cổng");
    fireEvent.change(port, { target: { value: "23000" } });
    await userEvent.click(screen.getByRole("button", { name: "Lưu API cục bộ" }));
    fireEvent.change(port, { target: { value: "24000" } });
    await act(async () => complete({ enabled: false, port: 23000, token: "" }));
    expect(port).toHaveValue(24000);
    expect(screen.getByRole("button", { name: "Lưu API cục bộ" })).toBeEnabled();
  });
});
