import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { resetConfirms } from "./confirmStore";
import { resetToasts } from "./toastStore";

vi.mock("./api", () => ({
  agentBulkRepair: vi.fn(async () => []),
  agentListStatuses: vi.fn(async () => []),
  authSession: vi.fn(async () => ({ showAuthUi: false, bypassed: true, user: null })),
  exampleScript: vi.fn(async () => "{}"),
  getStreamSettings: vi.fn(async () => ({
    fps: 24,
    tileSize: "medium",
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
  refreshDevices: vi.fn(async () => []),
  setStreamSettings: vi.fn(async (settings: unknown) => settings),
  startupError: vi.fn(async () => null),
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
    expect(screen.getByText("Flow", { selector: ".topbar-title" })).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Tác vụ" }));
    await user.click(await screen.findByRole("button", { name: "Bỏ thay đổi" }));
    await waitFor(() =>
      expect(screen.getByText("Tác vụ", { selector: ".topbar-title" })).toBeVisible(),
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
