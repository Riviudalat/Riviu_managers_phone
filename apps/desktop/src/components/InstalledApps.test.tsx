import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { InstalledApps } from "./InstalledApps";
import { listInstalledApps } from "../api";

vi.mock("../api", () => ({ listInstalledApps: vi.fn() }));

const listMock = vi.mocked(listInstalledApps);

beforeEach(() => listMock.mockReset());
afterEach(cleanup);

const REDMI = [
  { bundleId: "com.ss.android.ugc.trill", kind: "user" as const, label: null },
  { bundleId: "com.riviu.agent", kind: "user" as const, label: null },
  { bundleId: "com.android.settings", kind: "system" as const, label: null },
];

// The refusal path and every emptiness distinction live in installedAppsView.test.ts,
// which needs no promises. What is left here is the wiring: does the fetch happen, does
// the toggle reach the view, does pointing at another phone refetch.
describe("InstalledApps", () => {
  it("shows installed apps and hides system ones behind a counted toggle", async () => {
    listMock.mockResolvedValue(REDMI);
    render(<InstalledApps udid="10969614" deviceName="Redmi" />);

    await waitFor(() => expect(screen.getByText("com.riviu.agent")).toBeTruthy());
    // System is present in the data and counted, but not listed until asked for.
    expect(screen.queryByText("com.android.settings")).toBeNull();
    expect(screen.getByText(/Hiện app hệ thống \(1\)/)).toBeTruthy();

    await userEvent.click(screen.getByRole("checkbox"));

    expect(screen.getByText("com.android.settings")).toBeTruthy();
  });

  it("says the names are package names rather than pretending they are labels", async () => {
    // The honest-panel requirement: a null label must not read as an app called nothing.
    listMock.mockResolvedValue(REDMI);
    render(<InstalledApps udid="10969614" deviceName="Redmi" />);

    await waitFor(() => expect(screen.getByText(/đây là tên gói/)).toBeTruthy());
  });

  it("filters on the package name", async () => {
    listMock.mockResolvedValue(REDMI);
    render(<InstalledApps udid="10969614" deviceName="Redmi" />);
    await waitFor(() => expect(screen.getByText("com.riviu.agent")).toBeTruthy());

    await userEvent.type(screen.getByLabelText("Lọc ứng dụng"), "trill");

    expect(screen.getByText("com.ss.android.ugc.trill")).toBeTruthy();
    expect(screen.queryByText("com.riviu.agent")).toBeNull();
  });

  it("refetches when the panel is pointed at another phone", async () => {
    listMock.mockResolvedValue(REDMI);
    const { rerender } = render(<InstalledApps udid="10969614" deviceName="Redmi" />);
    await waitFor(() => expect(listMock).toHaveBeenCalledWith("10969614"));

    rerender(<InstalledApps udid="ce061716" deviceName="Note 8" />);

    await waitFor(() => expect(listMock).toHaveBeenCalledWith("ce061716"));
  });
});
