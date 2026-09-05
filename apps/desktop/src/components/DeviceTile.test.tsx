import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { DeviceTile } from "./DeviceTile";
import { deviceOperationalView } from "../deviceWork";
import type { DeviceInfo, TileStreamState } from "../types";

// The canvas does real OffscreenCanvas work and talks to the decode worker; none of that is
// what this file is about, and jsdom has neither.
vi.mock("./PhoneCanvas", () => ({
  PhoneCanvas: () => <div data-testid="phone-canvas" />,
}));

// No frames ever arrive here, which is precisely the state under test. `decodeFailed` is a
// let so one case can flip it without a second mock factory.
let decodeFailed = false;
vi.mock("../viewStore", () => ({
  useViewLive: () => false,
  useViewSize: () => undefined,
  useViewDecodeFailed: () => decodeFailed,
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

function renderTile(
  overrides: Partial<DeviceInfo> = {},
  owner: "nurture" | "interaction" | null = null,
  callbacks: {
    onSelect?: (udid: string, additive: boolean) => void;
    onOpen?: (udid: string) => void;
    onContextMenu?: (udid: string, x: number, y: number) => void;
  } = {},
) {
  const renderedDevice = device(overrides);
  return render(
    <DeviceTile
      device={renderedDevice}
      width={120}
      index={1}
      operational={deviceOperationalView(renderedDevice, owner)}
      selected={false}
      onSelect={callbacks.onSelect ?? (() => {})}
      onOpen={callbacks.onOpen ?? (() => {})}
      onPrepare={() => {}}
      onContextMenu={callbacks.onContextMenu}
    />,
  );
}

afterEach(() => {
  cleanup();
  decodeFailed = false;
});

describe("device tile, before any frame arrives", () => {
  it("leads with the fleet number and alias, then shows status and current owner", () => {
    renderTile({ name: "Điện thoại gốc", model: "SM-G955F" }, "nurture");

    expect(screen.getByText("Máy 1")).toBeVisible();
    expect(screen.getAllByText("Điện thoại gốc").length).toBeGreaterThan(0);
    expect(screen.getByText("Bận · Nuôi TikTok")).toBeVisible();
    expect(screen.queryByText("SM-G955F")).toBeNull();
    expect(screen.queryByText("device-1")).toBeNull();
    expect(screen.queryByText("ready")).toBeNull();
  });

  it.each<TileStreamState | undefined>([undefined, "parked", "sampling", "stale"])(
    "shows the loading mark without offering a stream retry for %s",
    (tileStreamState) => {
      // `parked` is the DEFAULT a device is listed with, not a decision to leave it stopped,
      // and the keeper starts a producer for every device it sees. Offering "Start" during
      // that told the operator their phone was idle and needed a nudge, which was never true.
      renderTile({ tileStreamState });

      expect(screen.getByRole("status")).toBeTruthy();
      expect(screen.queryByRole("button", { name: "Thử lại" })).toBeNull();
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

  it("stops breathing the loading mark once the codec has refused the stream", () => {
    // This tile spun the logo forever on an undecodable stream: `viewDecodeFailed` existed
    // and nothing read it. And there is deliberately NO retry -- every codec candidate was
    // already tried, so the button could not succeed.
    decodeFailed = true;
    renderTile({ tileStreamState: "sampling" });

    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.queryByRole("button", { name: "Thử lại" })).toBeNull();
    expect(screen.getByText(/không đọc được luồng này/i)).toBeTruthy();
  });

  it("keeps a phone whose cable dropped on the grid, with the reason it gave", () => {
    // The other half of D4. `adb devices` reporting a phone as `offline` used to be
    // discarded in the driver, so the tile simply vanished — no row, no reason, and
    // indistinguishable from a phone somebody had unplugged on purpose. The Rust side
    // now hands that phone a row and a sentence (`unusable_device`, driver.rs), and this
    // is the half that proves the sentence reaches the operator.
    //
    // The awkward part is the combination: the device is `disconnected` and no stream
    // ever started, so there is no `tileStreamState` to key off — only `lastError`. A
    // tile that keyed on the stream state alone would draw a spinner over a phone that
    // is not coming back.
    renderTile({
      status: "disconnected",
      wdaReady: false,
      lastError:
        "adb sees this device but it is not answering — check the cable or the USB hub, or wait if it is rebooting",
    });

    expect(screen.getByText(/not answering — check the cable or the USB hub, or wait/)).toBeTruthy();
    expect(screen.queryByRole("status"), "no spinner over a phone that is not coming back").toBeNull();
  });

  it("treats a reported error as a failure even when the state has not caught up", () => {
    // The two travel separately -- `lastError` is set by whatever failed, `tileStreamState`
    // by the next device poll. Between them the tile would otherwise spin over a phone that
    // has already given up.
    renderTile({ tileStreamState: "sampling", lastError: "scrcpy-server exited" });

    expect(screen.getByRole("button", { name: "Thử lại" })).toBeTruthy();
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("selects with Enter and Space and opens through a keyboard-operable action", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    const onOpen = vi.fn();
    renderTile({ name: "Kệ trên" }, null, { onSelect, onOpen });

    const tile = screen.getByRole("option", { name: /Máy 1.*Kệ trên/i });
    tile.focus();
    await user.keyboard("{Enter}");
    await user.keyboard(" ");
    expect(onSelect).toHaveBeenNthCalledWith(1, "device-1", false);
    expect(onSelect).toHaveBeenNthCalledWith(2, "device-1", false);

    await user.tab();
    expect(screen.getByRole("button", { name: "Mở màn hình Máy 1" })).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(onOpen).toHaveBeenCalledOnce();
    expect(onOpen).toHaveBeenCalledWith("device-1");
  });

  it("opens the device menu with the standard keyboard shortcut", async () => {
    const user = userEvent.setup();
    const onContextMenu = vi.fn();
    renderTile({ name: "Kệ trên" }, null, { onContextMenu });

    const tile = screen.getByRole("option", { name: /Máy 1.*Kệ trên/i });
    tile.focus();
    await user.keyboard("{Shift>}{F10}{/Shift}");

    expect(onContextMenu).toHaveBeenCalledOnce();
    expect(onContextMenu).toHaveBeenCalledWith("device-1", expect.any(Number), expect.any(Number));
  });
});
