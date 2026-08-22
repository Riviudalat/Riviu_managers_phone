import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as api from "./api";
import type { AppEvent } from "./types";
import { useFleet } from "./useFleet";

vi.mock("./api", () => ({
  listDevices: vi.fn(async () => []),
  listJobs: vi.fn(async () => []),
  listGroups: vi.fn(async () => []),
  listDeviceMetas: vi.fn(async () => []),
  driverDegradedReason: vi.fn(async () => null),
  androidUnavailableReason: vi.fn(async () => null),
  startupError: vi.fn(async () => null),
  retryStartup: vi.fn(async () => null),
  listenRiviuEvents: vi.fn(async () => () => undefined),
}));

const mocked = api as unknown as Record<string, ReturnType<typeof vi.fn>>;

/**
 * The fleet cluster, tested without mounting the application.
 *
 * That is the entire reason it moved out of `App.tsx`: the behaviour below is a rule about
 * one effect's dependency list, and reaching it used to mean rendering the whole shell,
 * every popup and the phone grid first. The rule itself is not hypothetical — the comment
 * on that dependency list records the session where it was broken.
 */
describe("useFleet", () => {
  beforeEach(() => {
    // `mockClear` keeps implementations, so every default is restated here. Without this a
    // roster set by one test leaks into the next, which is exactly what made the append
    // case below read as a failure when the code was right.
    for (const fn of Object.values(mocked)) fn.mockReset();
    mocked.listDevices.mockResolvedValue([]);
    mocked.listJobs.mockResolvedValue([]);
    mocked.listGroups.mockResolvedValue([]);
    mocked.listDeviceMetas.mockResolvedValue([]);
    mocked.driverDegradedReason.mockResolvedValue(null);
    mocked.androidUnavailableReason.mockResolvedValue(null);
    mocked.startupError.mockResolvedValue(null);
    mocked.retryStartup.mockResolvedValue(null);
    mocked.listenRiviuEvents.mockImplementation(async () => () => undefined);
  });

  it("subscribes once and loads the fleet when startup is clean", async () => {
    renderHook(() => useFleet());
    await waitFor(() => expect(mocked.listenRiviuEvents).toHaveBeenCalledTimes(1));
    expect(mocked.listDevices).toHaveBeenCalled();
  });

  it("subscribes to nothing while startup is blocked", async () => {
    mocked.startupError.mockResolvedValue("agent chưa cấu hình");
    const { result } = renderHook(() => useFleet());
    await waitFor(() => expect(result.current.startupIssue).toBe("agent chưa cấu hình"));
    expect(mocked.listenRiviuEvents).not.toHaveBeenCalled();
    expect(mocked.listDevices).not.toHaveBeenCalled();
  });

  it("a successful retry gets a subscription, not just a reload", async () => {
    // The bug this pins, in full: the retry button cleared the error and the app rendered,
    // but the boot effect never ran again — so nothing was subscribed for the rest of the
    // session and the grid moved only on the poll. The handler replayed `reload()` by hand
    // and could not replay the subscription.
    mocked.startupError.mockResolvedValue("agent chưa cấu hình");
    const { result } = renderHook(() => useFleet());
    await waitFor(() => expect(result.current.startupIssue).toBe("agent chưa cấu hình"));
    expect(mocked.listenRiviuEvents).not.toHaveBeenCalled();

    mocked.startupError.mockResolvedValue(null);
    await act(async () => {
      await result.current.retry();
    });

    await waitFor(() => expect(mocked.listenRiviuEvents).toHaveBeenCalledTimes(1));
    expect(result.current.startupIssue).toBeNull();
  });

  it("a retry that is still blocked does not subscribe", async () => {
    mocked.startupError.mockResolvedValue("agent chưa cấu hình");
    const { result } = renderHook(() => useFleet());
    await waitFor(() => expect(result.current.startupIssue).toBe("agent chưa cấu hình"));

    mocked.retryStartup.mockResolvedValue("vẫn chưa cấu hình");
    await act(async () => {
      await result.current.retry();
    });

    expect(result.current.startupIssue).toBe("vẫn chưa cấu hình");
    expect(mocked.listenRiviuEvents).not.toHaveBeenCalled();
  });

  it("keeps the phones when the group list fails", async () => {
    // Groups load outside the `Promise.all` on purpose: inside it, a group failure rejected
    // the whole reload and the grid blanked because a tab strip could not be drawn.
    mocked.listDevices.mockResolvedValue([{ udid: "a" }, { udid: "b" }]);
    mocked.listGroups.mockRejectedValue(new Error("no groups"));
    const { result } = renderHook(() => useFleet());
    await waitFor(() => expect(result.current.devices).toHaveLength(2));
    expect(result.current.groups).toEqual([]);
    expect(result.current.bootError).toBeNull();
  });

  it("applies a deviceUpdated event to the phone it names and no other", async () => {
    let emit: ((event: AppEvent) => void) | undefined;
    mocked.listenRiviuEvents.mockImplementation(async (handler: (e: AppEvent) => void) => {
      emit = handler;
      return () => undefined;
    });
    mocked.listDevices.mockResolvedValue([
      { udid: "a", name: "one" },
      { udid: "b", name: "two" },
    ]);
    const { result } = renderHook(() => useFleet());
    await waitFor(() => expect(result.current.devices).toHaveLength(2));

    act(() => {
      emit?.({
        type: "deviceUpdated",
        device: { udid: "b", name: "two renamed" },
      } as AppEvent);
    });

    expect(result.current.devices.map((d) => d.name)).toEqual(["one", "two renamed"]);
  });

  it("adds a phone the roster had not seen rather than dropping the event", async () => {
    let emit: ((event: AppEvent) => void) | undefined;
    mocked.listenRiviuEvents.mockImplementation(async (handler: (e: AppEvent) => void) => {
      emit = handler;
      return () => undefined;
    });
    const { result } = renderHook(() => useFleet());
    await waitFor(() => expect(mocked.listenRiviuEvents).toHaveBeenCalled());

    act(() => {
      emit?.({ type: "deviceUpdated", device: { udid: "new" } } as AppEvent);
    });

    expect(result.current.devices.map((d) => d.udid)).toEqual(["new"]);
  });
});
