import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { resetConfirms } from "./confirmStore";
import { resetToasts } from "./toastStore";
import type { DeviceInfo } from "./types";

vi.mock("./api", () => ({
  agentBulkRepair: vi.fn(async () => []),
  agentListStatuses: vi.fn(async () => []),
  // Both fleet-health probes are mocked explicitly. `driverDegradedReason` was absent
  // and survived only because the call site wraps it in `.catch` — an unmocked export is
  // `undefined`, so calling it throws synchronously and would have surfaced as a boot
  // error rather than as the missing mock it is.
  // Added with the bundled-tools banner. An export missing from this mock returns `undefined`,
  // and `.catch` on that throws synchronously inside the boot effect — the same silence that
  // took six interaction tests down at once.
  androidToolProblems: vi.fn(async () => [] as string[]),
  androidUnavailableReason: vi.fn(async () => null),
  appLogDirectory: vi.fn(async () => String.raw`D:\RiviuData\logs`),
  // The zoom overlay takes a control lease as it mounts. Missing from this mock, the import is
  // `undefined` and calling it throws — the trap this file already documents three times.
  deviceControlBegin: vi.fn(async () => undefined),
  deviceControlEnd: vi.fn(async () => undefined),
  // The health popup mounts on demand, but the doctrine above applies to every export
  // this file's components can reach: unmocked is `undefined`, and the first click would
  // throw synchronously instead of failing as the missing mock it is.
  deviceHealth: vi.fn(async () => ({
    udid: "fixture",
    agent: { state: "unknown" },
    notes: [],
  })),
  driverDegradedReason: vi.fn(async () => null),
  deploymentFrontendReady: vi.fn(async () => false),
  exampleScript: vi.fn(async () => "{}"),
  getStreamSettings: vi.fn(async () => ({
    fps: 24,
    gridQuality: "medium",
    focusQuality: "high",
  })),
  listenRiviuEvents: vi.fn(async () => () => undefined),
  listDevices: vi.fn(async () => []),
  listDeviceWorkStates: vi.fn(async () => []),
  // The grid reads the operator's own records (alias, number) on every reload. Mocked for
  // the reason the comment above gives, and this one bit: an unmocked export is `undefined`,
  // so the call threw *synchronously* inside `reload`'s try block — past the `.catch` that
  // was meant to make this failure cost only the labels — and the whole boot reported an
  // error instead. Three banner tests failed with nothing in them changed.
  listDeviceMetas: vi.fn(async () => []),
  getDeviceMeta: vi.fn(async (udid: string) => ({ udid, notes: "", tags: [] })),
  saveDeviceMeta: vi.fn(async () => undefined),
  listGroups: vi.fn(async () => []),
  listJobs: vi.fn(async () => []),
  listSchedules: vi.fn(async () => []),
  listScripts: vi.fn(async () => [["fixture", "{}"]]),
  prepareDevice: vi.fn(async () => undefined),
  viewEndpoint: vi.fn(async () => null),
  viewEnsure: vi.fn(async () => undefined),
  viewSetPreset: vi.fn(async () => undefined),
  saveViewSnapshot: vi.fn(async () => ""),
  refreshDevices: vi.fn(async () => []),
  setStreamSettings: vi.fn(async (settings: unknown) => settings),
  startupError: vi.fn(async () => null as string | null),
  retryStartup: vi.fn(async () => null as string | null),
  // Settings renders the update section on mount, but nothing checks on mount — the
  // resting state is "not asked". Mocked anyway: an unmocked export is `undefined`,
  // and the button's onClick would throw synchronously if anyone pressed it.
  updateCheck: vi.fn(async () => ({
    available: false,
    version: null,
    current: "0.1.0",
    busyReason: null,
  })),
  updateInstall: vi.fn(async () => undefined),
}));

vi.mock("./components/flow/FlowWorkspace", () => ({
  FlowWorkspace: ({ onDirtyChange }: { onDirtyChange: (dirty: boolean) => void }) => (
    <section aria-label="Flow fixture">
      <button type="button" onClick={() => onDirtyChange(true)}>Mark fixture dirty</button>
      <button type="button" onClick={() => onDirtyChange(false)}>Mark fixture clean</button>
    </section>
  ),
}));

vi.mock("./components/orchestration/OrchestrationWorkspace", () => ({
  OrchestrationWorkspace: ({
    onDirtyChange,
    targetRef,
  }: {
    onDirtyChange: (dirty: boolean) => void;
    targetRef?: { type: string };
  }) => (
    <section aria-label="Điều phối fixture" data-target-type={targetRef?.type}>
      <button type="button" onClick={() => onDirtyChange(true)}>Mark orchestration dirty</button>
    </section>
  ),
}));

vi.mock("./components/NurturePopup", () => ({
  NurturePopup: ({
    surface,
    targetUdids,
    targetRef,
  }: {
    surface?: string;
    targetUdids?: string[];
    targetRef?: { type: string };
  }) => (
    <section
      aria-label="Không gian Nuôi TikTok"
      data-surface={surface}
      data-targets={targetUdids?.join(",")}
      data-target-type={targetRef?.type}
    />
  ),
}));

vi.mock("./components/InteractionPopup", () => ({
  InteractionPopup: ({ surface }: { surface?: string }) => (
    <section aria-label="Không gian Tương tác" data-surface={surface} />
  ),
}));

afterEach(() => {
  cleanup();
  resetConfirms();
  resetToasts();
});

beforeEach(() => {
  vi.clearAllMocks();
});

const androidPhone: DeviceInfo = {
  udid: "10969614",
  name: "Redmi",
  model: "23021RAAEG",
  platform: "android",
  osVersion: "15",
  connection: "usb",
  status: "ready",
  wdaReady: false,
};

const iphone: DeviceInfo = {
  udid: "a99f4bd9f877b2a0e3682ee24fd1c68f75ba6982",
  name: "iPhone 8",
  model: "iPhone10,1",
  platform: "ios",
  osVersion: "16.7.15",
  connection: "usb",
  status: "ready",
  wdaReady: true,
};

describe("toolbar Start", () => {
  it("signals deployment readiness only after the first fleet load settles", async () => {
    const api = await import("./api");
    let resolveDevices: (devices: DeviceInfo[]) => void = () => undefined;
    vi.mocked(api.listDevices).mockImplementationOnce(
      () => new Promise((resolve) => { resolveDevices = resolve; }),
    );
    render(<App />);
    await waitFor(() => expect(api.startupError).toHaveBeenCalled());
    expect(api.deploymentFrontendReady).not.toHaveBeenCalled();

    resolveDevices([]);
    await waitFor(() => expect(api.deploymentFrontendReady).toHaveBeenCalledTimes(1));
  });

  it("uses viewEnsure on Android and does not call prepareDevice", async () => {
    const api = await import("./api");
    vi.mocked(api.listDevices).mockResolvedValue([androidPhone]);
    render(<App />);
    await waitFor(() => expect(screen.getByText("Redmi")).toBeInTheDocument());
    await userEvent.click(
      screen.getByTitle("Mở luồng xem cho các máy đã chọn hoặc toàn bộ danh sách"),
    );
    await waitFor(() => expect(api.viewEnsure).toHaveBeenCalledWith("10969614"));
    expect(api.prepareDevice).not.toHaveBeenCalled();
  });

  it("uses prepareDevice on iPhone and does not call viewEnsure", async () => {
    const api = await import("./api");
    vi.mocked(api.listDevices).mockResolvedValue([iphone]);
    render(<App />);
    await waitFor(() => expect(screen.getByText("iPhone 8")).toBeInTheDocument());
    await userEvent.click(
      screen.getByTitle("Mở luồng xem cho các máy đã chọn hoặc toàn bộ danh sách"),
    );
    await waitFor(() => expect(api.prepareDevice).toHaveBeenCalledWith(iphone.udid));
    expect(api.viewEnsure).not.toHaveBeenCalled();
  });
});

describe("device group scope", () => {
  it("filters the table with the same active group as the stream grid", async () => {
    const api = await import("./api");
    const other: DeviceInfo = {
      ...androidPhone,
      udid: "ce0617",
      name: "Note 8",
      model: "SM-N950F",
    };
    vi.mocked(api.listDevices).mockResolvedValue([androidPhone, other]);
    vi.mocked(api.listGroups).mockResolvedValue([
      {
        id: "group-redmi",
        name: "Máy Redmi",
        color: "#22c55e",
        udids: [androidPhone.udid],
        createdAt: "2026-09-03T00:00:00Z",
      },
    ]);

    render(<App />);
    await waitFor(() => expect(screen.getByText("Note 8")).toBeInTheDocument());
    await userEvent.click(screen.getByRole("tab", { name: /Máy Redmi/ }));
    await userEvent.click(screen.getByTitle("Danh sách"));

    expect(screen.getByRole("cell", { name: /Máy 1.*Redmi/ })).toBeInTheDocument();
    expect(screen.queryByRole("cell", { name: /Note 8/ })).toBeNull();
  });
});

describe("device operational identity", () => {
  it("shows the active work owner in the summary and keeps technical identity in details", async () => {
    const api = await import("./api");
    vi.mocked(api.listDevices).mockResolvedValue([androidPhone]);
    vi.mocked(api.listDeviceWorkStates).mockResolvedValue([
      { udid: androidPhone.udid, currentOwner: "interaction" },
    ]);

    render(<App />);
    await waitFor(() => expect(screen.getByText("Redmi")).toBeInTheDocument());
    await userEvent.click(screen.getByTitle("Danh sách"));

    expect(screen.getByText("Bận · Tương tác")).toBeVisible();
    expect(screen.queryByText(androidPhone.model)).toBeNull();
    expect(screen.queryByText(androidPhone.udid)).toBeNull();

    await userEvent.click(screen.getByRole("button", { name: "Xem chi tiết Máy 1" }));
    const details = screen.getByRole("dialog", { name: "Chi tiết Máy 1" });
    expect(within(details).getByText(androidPhone.model)).toBeVisible();
    expect(within(details).getByText(androidPhone.udid)).toBeVisible();
    expect(within(details).getByText("Tương tác")).toBeVisible();
    expect(within(details).getByText("ready")).toBeVisible();
  });

  it("never calls a ready phone idle when owner lookup fails and retries in place", async () => {
    const api = await import("./api");
    vi.mocked(api.listDevices).mockResolvedValue([androidPhone]);
    vi.mocked(api.listDeviceWorkStates).mockRejectedValue(new Error("owner projection offline"));

    render(<App />);

    expect(await screen.findByText("Chưa đọc được tác vụ")).toBeVisible();
    const tile = screen.getByTestId("device-tile");
    expect(within(tile).queryByText("Sẵn sàng")).toBeNull();
    const alert = screen.getByRole("alert");
    expect(within(alert).getByText("Không đọc được tác vụ đang chạy trên thiết bị")).toBeVisible();
    expect(within(alert).getByText("owner projection offline")).toBeVisible();

    vi.mocked(api.listDeviceWorkStates).mockResolvedValue([]);
    await userEvent.click(within(alert).getByRole("button", { name: "Thử lại" }));

    expect(await within(tile).findByText("Sẵn sàng")).toBeVisible();
    await waitFor(() =>
      expect(screen.queryByText("Chưa đọc được tác vụ")).not.toBeInTheDocument(),
    );
  });

  it("uses one search and status filter for grid and list without renumbering the fleet", async () => {
    const api = await import("./api");
    const warningPhone: DeviceInfo = {
      ...androidPhone,
      udid: "warning-phone",
      name: "Kệ giữa",
      status: "error",
    };
    const busyPhone: DeviceInfo = {
      ...androidPhone,
      udid: "busy-phone",
      name: "Kệ cuối",
    };
    vi.mocked(api.listDevices).mockResolvedValue([androidPhone, warningPhone, busyPhone]);
    vi.mocked(api.listDeviceWorkStates).mockResolvedValue([
      { udid: busyPhone.udid, currentOwner: "nurture" },
    ]);

    render(<App />);
    await waitFor(() => expect(screen.getAllByTestId("device-tile")).toHaveLength(3));
    await waitFor(() => expect(screen.getByText("Bận · Nuôi TikTok")).toBeVisible());

    await userEvent.type(screen.getByRole("searchbox", { name: "Tìm thiết bị" }), "ke cuoi");
    expect(screen.getAllByTestId("device-tile")).toHaveLength(1);
    expect(screen.getByText("Máy 3")).toBeVisible();
    expect(screen.getAllByText("Kệ cuối").length).toBeGreaterThan(0);

    await userEvent.click(screen.getByTitle("Danh sách"));
    expect(screen.getByRole("cell", { name: /Máy 3.*Kệ cuối/ })).toBeVisible();
    expect(screen.queryByRole("cell", { name: /Máy 1.*Redmi/ })).toBeNull();

    await userEvent.clear(screen.getByRole("searchbox", { name: "Tìm thiết bị" }));
    await userEvent.selectOptions(screen.getByRole("combobox", { name: "Trạng thái thiết bị" }), "warning");
    expect(screen.getByRole("cell", { name: /Máy 2.*Kệ giữa/ })).toBeVisible();
    expect(screen.queryByRole("cell", { name: /Máy 3.*Kệ cuối/ })).toBeNull();

    await userEvent.click(screen.getByTitle("Cửa sổ stream"));
    const [visibleTile] = screen.getAllByTestId("device-tile");
    expect(screen.getAllByTestId("device-tile")).toHaveLength(1);
    expect(screen.getByText("Máy 2")).toBeVisible();
    expect(within(visibleTile).getByText("Cần xem")).toBeVisible();
  });
});

describe("automation target resolution", () => {
  it("keeps an empty group at zero targets instead of expanding it to the fleet", async () => {
    const api = await import("./api");
    vi.mocked(api.listDevices).mockResolvedValue([androidPhone]);
    vi.mocked(api.listGroups).mockResolvedValue([
      {
        id: "empty",
        name: "Ca trống",
        color: "#888888",
        udids: ["departed"],
        createdAt: "2026-09-03T00:00:00Z",
      },
    ]);
    render(<App />);

    await waitFor(() => expect(screen.getByText("Redmi")).toBeInTheDocument());
    await userEvent.click(screen.getByRole("button", { name: "Nuôi TikTok" }));
    await userEvent.click(screen.getByRole("radio", { name: "Nhóm" }));

    const workspace = screen.getByRole("region", { name: "Không gian Nuôi TikTok" });
    expect(workspace).toHaveAttribute("data-targets", "");
    expect(workspace).toHaveAttribute("data-target-type", "group");
    expect(screen.getByRole("status")).toHaveTextContent("Ca trống · 0 máy");
  });
});

describe("the removed local login", () => {
  // Passwords were stored and compared as plaintext, so the login was removed rather than
  // patched. These are the two things a user could still see afterwards if the removal had
  // stopped at the backend: a nav entry to an account page with nothing behind it, and an
  // `auth_session` call on boot deciding whether to gate the app.
  it("leaves no account entry in the sidebar", async () => {
    const api = await import("./api");
    vi.mocked(api.listDevices).mockResolvedValue([androidPhone]);
    render(<App />);
    await waitFor(() => expect(screen.getByText("Redmi")).toBeInTheDocument());
    expect(screen.queryByRole("button", { name: "Tài khoản" })).toBeNull();
  });

  it("boots straight to the fleet without asking the backend about a session", async () => {
    const api = await import("./api");
    vi.mocked(api.listDevices).mockResolvedValue([androidPhone]);
    // The mock factory has no `authSession`, so an import of it is `undefined` and calling
    // it throws. Reaching the grid at all is the evidence that nothing calls it.
    expect("authSession" in api).toBe(false);
    render(<App />);
    await waitFor(() => expect(screen.getByText("Redmi")).toBeInTheDocument());
    expect(screen.queryByLabelText(/mật khẩu/i)).toBeNull();
  });
});

describe("a per-phone panel whose phone leaves the fleet", () => {
  /** The second phone matters: an empty roster is a failed scan, not a departure. */
  const other: DeviceInfo = { ...androidPhone, udid: "ce0617", name: "Note 8" };

  /** Right-click the Redmi tile and click a row of its menu. */
  async function openFromTileMenu(row: string) {
    const tile = await waitFor(() => {
      const found = document.querySelector('[data-udid="10969614"]');
      if (!found) throw new Error("no tile yet");
      return found;
    });
    fireEvent.contextMenu(tile);
    // The rows carry , not  —  sets menu
    // semantics for the context menu ().
    await userEvent.click(await screen.findByRole("menuitem", { name: row }));
  }

  afterEach(async () => {
    // **Put the roster back.** `vi.clearAllMocks()` in the file's `beforeEach` clears calls but
    // **not** implementations, so a `mockResolvedValue` set here survives into the next test —
    // and eighteen tests in this file reach for "Redmi" without setting the mock themselves.
    // Leaving `[Note 8]` behind failed one of them, in a test that had not changed.
    const api = await import("./api");
    vi.mocked(api.listDevices).mockResolvedValue([androidPhone]);
  });

  /** Swap the roster and press the header's refresh, which reloads it. */
  async function rosterBecomes(devices: DeviceInfo[]) {
    const api = await import("./api");
    vi.mocked(api.listDevices).mockResolvedValue(devices);
    await userEvent.click(screen.getByTitle("Làm mới danh sách máy"));
  }

  /**
   * **The operator's bug, as a test.** Before this, the panel unmounted silently when the
   * roster churned and the stale udid stayed in state — so clicking the same phone's row was a
   * `setState` with the value already there, React bailed out, and the row did nothing at all,
   * for that phone, until the app restarted. "Mở thư mục máy điện thoại còn mở không được."
   *
   * Part 3 is the assertion that would have failed.
   */
  it("closes the file browser out loud and lets it be reopened", async () => {
    const row = "Tệp trên máy…";
    const dialog = "Tệp trên Redmi";
    const label = "trình quản lý tệp";
    const api = await import("./api");
    vi.mocked(api.listDevices).mockResolvedValue([androidPhone, other]);
    render(<App />);
    await waitFor(() => expect(screen.getByText("Redmi")).toBeInTheDocument());

    await openFromTileMenu(row);
    expect(await screen.findByRole("dialog", { name: dialog })).toBeInTheDocument();

    // 1. the phone goes away and the panel closes
    await rosterBecomes([other]);
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: dialog })).toBeNull(),
    );
    // 2. and it says which phone, and what it closed — silence is what made this a bug report
    expect(await screen.findByText(new RegExp(`Redmi.*${label}`))).toBeInTheDocument();

    // 3. and the row works again when the phone comes back
    await rosterBecomes([androidPhone, other]);
    await waitFor(() => expect(screen.getByText("Redmi")).toBeInTheDocument());
    await openFromTileMenu(row);
    expect(await screen.findByRole("dialog", { name: dialog })).toBeInTheDocument();
  });

  /**
   * **A roster that reads empty is a failed scan, not a departure — and it recovers by itself.**
   *
   * `list_devices` reads until two consecutive `adb devices` agree, and a restarting adb server
   * can answer once with nothing. The panel does hide while the roster is empty, because the
   * render still resolves the phone out of it — but the udid is **kept**, so the panel comes
   * back on its own when the scan recovers, with no click and no toast. That is the whole
   * difference between a blip and a departure, and the guard in `surfaceDeparted` is what
   * draws it.
   */
  it("treats an empty roster as a blip and restores the panel by itself", async () => {
    const api = await import("./api");
    vi.mocked(api.listDevices).mockResolvedValue([androidPhone]);
    render(<App />);
    await waitFor(() => expect(screen.getByText("Redmi")).toBeInTheDocument());

    await openFromTileMenu("Tệp trên máy…");
    expect(await screen.findByRole("dialog", { name: "Tệp trên Redmi" })).toBeInTheDocument();

    await rosterBecomes([]);
    // Nobody was told a phone left, because none did.
    expect(screen.queryByText(/không còn kết nối/)).toBeNull();

    // And the panel returns without the operator touching anything.
    await rosterBecomes([androidPhone]);
    expect(await screen.findByRole("dialog", { name: "Tệp trên Redmi" })).toBeInTheDocument();
  });
});

describe("the zoom overlay per-phone rows", () => {
  /**
   * **The second half of "mở không được": the trigger lived on a page the panel did not.**
   *
   * `FocusStream` — the zoom overlay — is mounted outside `{page === "control"}`, and its
   * function list offers "Tệp trên máy…". While the panel itself was rendered *inside* that
   * block, clicking the row from the overlay on any other page set the udid, rendered nothing,
   * and then the stale udid made the row dead for that phone. Both popups now live beside
   * `FocusStream`, which is the surface that opens them.
   */
  /// One page, not every page, and that is enough: the defect was a single render sitting
  /// inside `{page === "control" && (`, so any page that is not "control" exercises it. "Tác
  /// vụ" is the cheapest — it reads `listJobs`, which this file already mocks, where the
  /// content and app pages would each drag in their own api surface for no extra coverage.
  it("opens the file browser from the zoom overlay while on another page", async () => {
    const api = await import("./api");
    vi.mocked(api.listDevices).mockResolvedValue([androidPhone]);
    render(<App />);
    await waitFor(() => expect(screen.getByText("Redmi")).toBeInTheDocument());

    // Double-click the tile to zoom into it, then leave the control page.
    const tile = document.querySelector('[data-udid="10969614"]');
    if (!tile) throw new Error("no tile");
    fireEvent.doubleClick(tile);
    await userEvent.click(screen.getByRole("button", { name: "Tác vụ" }));

    // The overlay's rows are plain buttons there rather than menu items, so reach for the
    // `title` both renderings share.
    await userEvent.click(await screen.findByTitle("Tệp trên máy…"));
    expect(
      await screen.findByRole("dialog", { name: "Tệp trên Redmi" }),
    ).toBeInTheDocument();
  });
});

describe("fleet health banners", () => {
  it("says why the Android half of the fleet is absent", async () => {
    // The command existed and was registered from the start; nothing called it, so an
    // Android phone that failed to join simply did not appear and gave no reason. This is
    // the assertion that keeps the caller wired.
    const api = await import("./api");
    vi.mocked(api.androidUnavailableReason).mockResolvedValueOnce(
      "adb is not usable (adb.exe): program not found",
    );
    render(<App />);
    await waitFor(() =>
      expect(screen.getByText(/adb is not usable/)).toBeInTheDocument(),
    );
    // Says out loud that it is a boot snapshot: `MultiplexDriver::new` fixes the backend
    // list at construction, so installing adb now needs a restart to take effect.
    expect(screen.getByText(/khởi động lại app/)).toBeInTheDocument();
  });

  it("says the phones will list and still not be drivable when the bundle is broken", async () => {
    // **Reported from a real install:** "lên app rồi, nhận điện thoại rồi, nhưng điều khiển
    // không được", with nothing on screen. Nine files are verified against
    // `android-tools-manifest.json` at boot and adb is only one of them, so a bundle that lost
    // the agent APKs still resolves adb — the fleet lists phones and every attempt to drive one
    // fails. The only record was a `log::warn!` in a file the operator did not know existed.
    const api = await import("./api");
    vi.mocked(api.androidToolProblems).mockResolvedValueOnce([
      "noarch/appium-uiautomator2-server.apk: sha256 mismatch",
    ]);
    render(<App />);
    await waitFor(() =>
      expect(screen.getByText(/sha256 mismatch/)).toBeInTheDocument(),
    );
    // The sentence that matters: not "no phones", which is the other banner and the wrong
    // answer — it sends the operator to look at adb, the one file that did work.
    expect(screen.getByText(/điều khiển sẽ không chạy/)).toBeInTheDocument();
    // The path comes from Tauri's active identifier, so Full and base builds cannot drift.
    expect(screen.getByText(String.raw`D:\RiviuData\logs`)).toBeInTheDocument();
    expect(screen.queryByText(/com\.riviu\.manager/)).not.toBeInTheDocument();
  });

  it("shows no bundled-tools banner when the bundle verifies", async () => {
    // The healthy answer is an empty list, and a banner that is always on is a banner nobody
    // reads — the same reason the Android-absent banner is `warn` and not `error`.
    render(<App />);
    await waitFor(() => expect(screen.getByText("Redmi")).toBeInTheDocument());
    expect(screen.queryByText(/điều khiển sẽ không chạy/)).toBeNull();
  });

  it("shows no Android banner on a farm whose Android half is fine", async () => {
    // A farm with no Android phones is the common case and is not a fault. A banner that
    // is always on is a banner nobody reads.
    render(<App />);
    await waitFor(() => expect(screen.queryByText(/adb/i)).not.toBeInTheDocument());
  });

  it("keeps an iOS failure scoped without claiming the Android fleet is empty", async () => {
    // Android startup is independent. A broken iOS sidecar may explain absent iPhones, but
    // it must not turn a healthy Android-only install into a global backend failure.
    const api = await import("./api");
    vi.mocked(api.listDevices).mockResolvedValue([androidPhone]);
    vi.mocked(api.driverDegradedReason).mockResolvedValueOnce("sidecar iOS bị suy giảm");
    render(<App />);
    await waitFor(() =>
      expect(screen.getByText(/sidecar iOS bị suy giảm/)).toBeInTheDocument(),
    );
    await waitFor(() => expect(screen.getByText("Redmi")).toBeInTheDocument());
    expect(screen.queryByText(/danh sách sẽ luôn trống/)).not.toBeInTheDocument();
    expect(screen.getByText(/Nhánh iOS không sẵn sàng/)).toBeInTheDocument();
    expect(screen.queryByText(/khởi động lại app/)).not.toBeInTheDocument();
  });
});

describe("Flow page integration", () => {
  it("uses the topbar as the one semantic page heading", async () => {
    const user = userEvent.setup();
    render(<App />);

    expect(screen.getByRole("heading", { level: 1, name: "Thiết bị" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Dữ liệu" }));
    expect(screen.getByRole("heading", { level: 1, name: "Dữ liệu" })).toBeVisible();
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
  });

  it("prompts once before leaving a dirty Flow draft", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(screen.getByRole("button", { name: "Flow" }));
    await user.click(
      await screen.findByRole("button", { name: "Mark fixture dirty" }),
    );
    await user.click(screen.getByRole("button", { name: "Tác vụ" }));

    // Declining the themed confirm keeps the draft open on the Flow page.
    await user.click(await screen.findByRole("button", { name: "Ở lại" }));
    await waitFor(() =>
      expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument(),
    );
    expect(screen.getByText("Flow", { selector: "[data-testid='page-title']" })).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Tác vụ" }));
    await user.click(await screen.findByRole("button", { name: "Bỏ thay đổi" }));
    await waitFor(() =>
      expect(screen.getByText("Tác vụ", { selector: "[data-testid='page-title']" })).toBeVisible(),
    );
  });

  it("switches between device Flow and orchestration", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(screen.getByRole("button", { name: "Flow" }));
    expect(screen.getByRole("tab", { name: "Flow thiết bị" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await user.click(screen.getByRole("tab", { name: "Điều phối" }));
    expect(screen.getByRole("tab", { name: "Điều phối" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("group", { name: "Phạm vi thiết bị" })).toBeVisible();
    expect(await screen.findByRole("region", { name: "Điều phối fixture" })).toHaveAttribute(
      "data-target-type",
      "all",
    );
  });

  it("links both Flow modes to panels and activates them with the horizontal keyboard pattern", async () => {
    render(<App />);
    await userEvent.click(screen.getByRole("button", { name: "Flow" }));

    const device = screen.getByRole("tab", { name: "Flow thiết bị" });
    const orchestration = screen.getByRole("tab", { name: "Điều phối" });
    expect(device).toHaveAttribute("tabindex", "0");
    expect(orchestration).toHaveAttribute("tabindex", "-1");
    for (const tab of [device, orchestration]) {
      const panel = document.getElementById(tab.getAttribute("aria-controls")!);
      expect(panel).toHaveAttribute("role", "tabpanel");
      expect(panel).toHaveAttribute("aria-labelledby", tab.id);
    }

    device.focus();
    fireEvent.keyDown(device, { key: "ArrowRight" });
    await waitFor(() => expect(orchestration).toHaveAttribute("aria-selected", "true"));
    expect(orchestration).toHaveFocus();

    fireEvent.keyDown(orchestration, { key: "Home" });
    await waitFor(() => expect(device).toHaveAttribute("aria-selected", "true"));
    expect(device).toHaveFocus();
    fireEvent.keyDown(device, { key: "End" });
    await waitFor(() => expect(orchestration).toHaveFocus());
    fireEvent.keyDown(orchestration, { key: "ArrowLeft" });
    await waitFor(() => expect(device).toHaveFocus());
  });

  it("registers a close guard only while the Flow draft is dirty", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(screen.getByRole("button", { name: "Flow" }));
    const cleanEvent = new Event("beforeunload", { cancelable: true });
    fireEvent(window, cleanEvent);
    expect(cleanEvent.defaultPrevented).toBe(false);

    await user.click(
      await screen.findByRole("button", { name: "Mark fixture dirty" }),
    );
    await waitFor(() => {
      const dirtyEvent = new Event("beforeunload", { cancelable: true });
      fireEvent(window, dirtyEvent);
      expect(dirtyEvent.defaultPrevented).toBe(true);
    });

    await user.click(screen.getByRole("button", { name: "Mark fixture clean" }));
    await waitFor(() => {
      const cleanAgain = new Event("beforeunload", { cancelable: true });
      fireEvent(window, cleanAgain);
      expect(cleanAgain.defaultPrevented).toBe(false);
    });
  });
});

describe("fleet diagnostics page integration", () => {
  it("opens the read-only diagnostics surface from navigation", async () => {
    const api = await import("./api");
    vi.mocked(api.listDevices).mockResolvedValue([androidPhone]);
    render(<App />);
    await waitFor(() => expect(screen.getByText("Redmi")).toBeInTheDocument());

    await userEvent.click(screen.getByRole("button", { name: "Chẩn đoán" }));

    expect(screen.getByRole("heading", { level: 1, name: "Chẩn đoán" })).toBeVisible();
    expect(await screen.findByRole("region", { name: "Chẩn đoán fleet" })).toBeVisible();
    expect(api.deviceHealth).toHaveBeenCalledWith(androidPhone.udid);
  });

  it("opens nurture and interaction as dedicated workspaces", async () => {
    const user = userEvent.setup();
    render(<App />);

    const deviceToolbar = document.querySelector(".profile-toolbar");
    expect(deviceToolbar).not.toBeNull();
    expect(deviceToolbar).not.toHaveTextContent("Nuôi TT");
    expect(deviceToolbar).not.toHaveTextContent("Tương tác");

    await user.click(screen.getByRole("button", { name: "Nuôi TikTok" }));
    expect(screen.getByRole("heading", { level: 1, name: "Nuôi TikTok" })).toBeVisible();
    expect(screen.getByRole("region", { name: "Không gian Nuôi TikTok" })).toHaveAttribute(
      "data-surface",
      "page",
    );
    expect(screen.getByRole("group", { name: "Phạm vi thiết bị" })).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Tương tác" }));
    expect(screen.getByRole("heading", { level: 1, name: "Tương tác" })).toBeVisible();
    expect(screen.getByRole("region", { name: "Không gian Tương tác" })).toHaveAttribute(
      "data-surface",
      "page",
    );
    expect(screen.getByRole("group", { name: "Phạm vi thiết bị" })).toBeVisible();
  });
});

describe("buttons that used to fail in silence", () => {
  it("says why Refresh did not refresh", async () => {
    // The toolbar's Refresh had no failure path at all: `onClick={() => void onRefresh()}`
    // dropped the rejection, so a scan that failed left the fleet unchanged and reported
    // nothing. Pressing it again did the same nothing.
    const api = await import("./api");
    vi.mocked(api.listDevices).mockResolvedValue([androidPhone]);
    vi.mocked(api.refreshDevices).mockRejectedValueOnce(new Error("adb server is not running"));
    render(<App />);
    await waitFor(() => expect(screen.getByText("Redmi")).toBeInTheDocument());

    await userEvent.click(screen.getByTitle("Quét lại thiết bị"));

    expect(await screen.findByText("Không làm mới được danh sách máy")).toBeInTheDocument();
    expect(await screen.findByText("adb server is not running")).toBeInTheDocument();
  });

  it("reports the phones that did not start, and does not call a partial start a success", async () => {
    // `Promise.all` over the Android half meant the first refusal ended the turn of every
    // device behind it, and the toast named only that one. This is the mixed-fleet case:
    // one phone refuses, the other starts, and the operator is told exactly that.
    const api = await import("./api");
    const second = { ...androidPhone, udid: "ce0617", name: "Note 8" };
    vi.mocked(api.listDevices).mockResolvedValue([androidPhone, second]);
    vi.mocked(api.viewEnsure).mockImplementation(async (udid: string) => {
      if (udid === androidPhone.udid) throw new Error("device offline");
    });
    render(<App />);
    await waitFor(() => expect(screen.getByText("Note 8")).toBeInTheDocument());

    await userEvent.click(
      screen.getByTitle("Mở luồng xem cho các máy đã chọn hoặc toàn bộ danh sách"),
    );

    expect(await screen.findByText("Khởi động 1/2 máy")).toBeInTheDocument();
    // The healthy phone was still reached, which is the half that used to be skipped.
    expect(api.viewEnsure).toHaveBeenCalledWith("ce0617");
  });

  it("says why a tile's Thử lại did not start the stream", async () => {
    // This is the button on a tile that has already failed once, so it is pressed at the
    // moment an operator can least tolerate silence -- and `void startDevicePreview(...)`
    // dropped the rejection into the console. Pressing it produced nothing, forever.
    const api = await import("./api");
    const failed = { ...androidPhone, tileStreamState: "error" as const, lastError: "no frames" };
    vi.mocked(api.listDevices).mockResolvedValue([failed]);
    vi.mocked(api.viewEnsure).mockRejectedValueOnce(new Error("scrcpy server refused"));
    render(<App />);

    await userEvent.click(await screen.findByRole("button", { name: "Thử lại" }));

    expect(await screen.findByText("Không mở lại được Redmi")).toBeInTheDocument();
    expect(await screen.findByText("scrcpy server refused")).toBeInTheDocument();
  });
});

describe("the startup failure screen", () => {
  it("actually retries the bootstrap instead of reloading the same stored error", async () => {
    // The button called `window.location.reload()`. The WebView came back, asked
    // `startup_error` again, and was handed the sentence stored at setup -- `bootstrap` had
    // run once and would never run again. An operator who fixed the cause (plugged in adb,
    // started the sidecar) had no way to tell the app short of quitting it.
    const api = await import("./api");
    vi.mocked(api.startupError).mockResolvedValueOnce("adb is not on PATH");
    vi.mocked(api.retryStartup).mockResolvedValueOnce(null);
    render(<App />);
    expect(await screen.findByText("adb is not on PATH")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Thử lại" }));

    expect(api.retryStartup).toHaveBeenCalledTimes(1);
    // The failure screen is gone, which is the whole point: the app is up.
    await waitFor(() =>
      expect(screen.queryByText("Chưa sẵn sàng khởi động")).not.toBeInTheDocument(),
    );
  });

  it("subscribes to fleet events once the retry has cleared the problem", async () => {
    // The boot effect returns early when startup failed, so nothing is listening —
    // correct. It then never ran again: its deps did not include the issue, and the
    // retry handler replayed `reload()` by hand with a comment saying the effect had
    // already run. It could not replay the subscription. So a window that came up
    // through this button spent the rest of the session with no `devicesUpdated`, no
    // `deviceUpdated`, no `jobUpdated` and no `streamFrame`: the grid only moved on the
    // three-second poll and no tile ever learned that a frame had arrived.
    const api = await import("./api");
    vi.mocked(api.startupError).mockResolvedValueOnce("adb is not on PATH");
    vi.mocked(api.retryStartup).mockResolvedValueOnce(null);
    render(<App />);
    expect(await screen.findByText("adb is not on PATH")).toBeInTheDocument();
    expect(
      api.listenRiviuEvents,
      "nothing should be listening while startup is blocked",
    ).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("button", { name: "Thử lại" }));

    await waitFor(() => expect(api.listenRiviuEvents).toHaveBeenCalled());
  });
  it("shows what is still wrong when the retry finds the same problem", async () => {
    const api = await import("./api");
    vi.mocked(api.startupError).mockResolvedValueOnce("adb is not on PATH");
    vi.mocked(api.retryStartup).mockResolvedValueOnce("adb is still not on PATH");
    render(<App />);
    expect(await screen.findByText("adb is not on PATH")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Thử lại" }));

    expect(await screen.findByText("adb is still not on PATH")).toBeInTheDocument();
  });
});
