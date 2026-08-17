import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { DeviceTile } from "./DeviceTile";
import type { DeviceInfo, TileStreamState } from "../types";

// The canvas does real OffscreenCanvas work and talks to the decode worker; none of that is
// what this file is about, and jsdom has neither.
vi.mock("./PhoneCanvas", () => ({
  PhoneCanvas: () => <div data-testid="phone-canvas" />,
}));

// No frames ever arrive here, which is precisely the state under test.
vi.mock("../viewStore", () => ({
  useViewLive: () => false,
}));

function device(overrides: Partial<DeviceInfo> = {}): DeviceInfo {
  return {
    udid: "device-1",
    name: "SM-G955F",
    model: "SM-G955F",
    platform: "android",
    osVersion: "9",
    connection: "usb",
    status: "ready",
    wdaReady: true,
    ...overrides,
  };
}

function renderTile(overrides: Partial<DeviceInfo> = {}) {
  return render(
    <DeviceTile
      device={device(overrides)}
      width={120}
      index={1}
      selected={false}
      onSelect={() => {}}
      onOpen={() => {}}
      onPrepare={() => {}}
    />,
  );
}

afterEach(cleanup);

describe("device tile, before any frame arrives", () => {
  it.each<TileStreamState | undefined>([undefined, "parked", "sampling", "stale"])(
    "shows the loading mark and nothing to press for %s",
    (tileStreamState) => {
      // `parked` is the DEFAULT a device is listed with, not a decision to leave it stopped,
      // and the keeper starts a producer for every device it sees. Offering "Start" during
      // that told the operator their phone was idle and needed a nudge, which was never true.
      renderTile({ tileStreamState });

      expect(screen.getByRole("status")).toBeTruthy();
      expect(screen.queryByRole("button")).toBeNull();
      expect(screen.queryByText(/No stream/i)).toBeNull();
    },
  );

  it("asks the operator to act only when the stream actually failed", () => {
    renderTile({ tileStreamState: "error", lastError: "adb: device unauthorized" });

    expect(screen.getByText("adb: device unauthorized")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Thử lại" })).toBeTruthy();
    // A spinner next to a failure would promise a recovery nobody is attempting.
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("treats a reported error as a failure even when the state has not caught up", () => {
    // The two travel separately -- `lastError` is set by whatever failed, `tileStreamState`
    // by the next device poll. Between them the tile would otherwise spin over a phone that
    // has already given up.
    renderTile({ tileStreamState: "sampling", lastError: "scrcpy-server exited" });

    expect(screen.getByRole("button", { name: "Thử lại" })).toBeTruthy();
    expect(screen.queryByRole("status")).toBeNull();
  });
});
