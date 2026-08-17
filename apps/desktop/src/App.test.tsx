import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
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
  androidUnavailableReason: vi.fn(async () => null),
  driverDegradedReason: vi.fn(async () => null),
  exampleScript: vi.fn(async () => "{}"),
  getStreamSettings: vi.fn(async () => ({
    fps: 24,
    gridQuality: "medium",
    focusQuality: "high",
  })),
  listenRiviuEvents: vi.fn(async () => () => undefined),
  listDevices: vi.fn(async () => []),
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
  startupError: vi.fn(async () => null),
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
  it("uses viewEnsure on Android and does not call prepareDevice", async () => {
    const api = await import("./api");
    vi.mocked(api.listDevices).mockResolvedValue([androidPhone]);
    render(<App />);
    await waitFor(() => expect(screen.getByText("Redmi")).toBeInTheDocument());
    await userEvent.click(
      screen.getByTitle("Prepare / start stream (selected hoặc tất cả)"),
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
      screen.getByTitle("Prepare / start stream (selected hoặc tất cả)"),
    );
    await waitFor(() => expect(api.prepareDevice).toHaveBeenCalledWith(iphone.udid));
    expect(api.viewEnsure).not.toHaveBeenCalled();
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

  it("shows no Android banner on a farm whose Android half is fine", async () => {
    // A farm with no Android phones is the common case and is not a fault. A banner that
    // is always on is a banner nobody reads.
    render(<App />);
    await waitFor(() => expect(screen.queryByText(/adb/i)).not.toBeInTheDocument());
  });

  it("keeps the iOS sidecar failure a separate, louder message", async () => {
    // Two different facts with two different fixes. Collapsing them into one string sends
    // the operator looking in the wrong place; the iOS one is an `error` banner because an
    // empty fleet there really is broken.
    const api = await import("./api");
    vi.mocked(api.driverDegradedReason).mockResolvedValueOnce("sidecar iOS bị suy giảm");
    render(<App />);
    await waitFor(() =>
      expect(screen.getByText(/sidecar iOS bị suy giảm/)).toBeInTheDocument(),
    );
    expect(screen.queryByText(/khởi động lại app/)).not.toBeInTheDocument();
  });
});

describe("Flow page integration", () => {
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

  it("keeps the legacy automation surface reachable", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(screen.getByRole("button", { name: "Flow" }));
    expect(screen.getByRole("tab", { name: "Flow" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await user.click(screen.getByRole("tab", { name: "Legacy" }));
    expect(screen.getByRole("tab", { name: "Legacy" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("heading", { name: "Kịch bản" })).toBeVisible();
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
