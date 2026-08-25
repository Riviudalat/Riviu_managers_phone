import { render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { DeviceTile } from "./components/DeviceTile";
import { FocusStream } from "./components/FocusStream";
import type { DeviceInfo } from "./types";

vi.mock("./api", () => ({
  backupDevice: vi.fn(),
  deviceControlBegin: vi.fn(async () => undefined),
  deviceControlEnd: vi.fn(async () => undefined),
  deviceKey: vi.fn(),
  deviceShell: vi.fn(async () => ({ exitCode: 0, stdout: "", stderr: "" })),
  deviceSwipe: vi.fn(),
  deviceSwipePath: vi.fn(),
  deviceTap: vi.fn(),
  deviceTypeText: vi.fn(),
  exportMedia: vi.fn(async () => 0),
  groupInput: vi.fn(),
  importMedia: vi.fn(async () => ""),
  installIpa: vi.fn(),
  setScreenRotation: vi.fn(async () => 0),
  viewRequestKeyframe: vi.fn(async () => true),
  rebootDevice: vi.fn(),
  restoreDevice: vi.fn(),
  saveViewSnapshot: vi.fn(),
  screenshot: vi.fn(),
}));

const fixture: DeviceInfo = {
  udid: "ce06",
  name: "Note 8",
  model: "SM-N950F",
  platform: "android",
  osVersion: "8.0",
  connection: "usb",
  status: "ready",
  wdaReady: true,
  tileStreamState: "live",
};

describe("tile and overlay paint path", () => {
  it("does not put a JPEG data URL on the tile or overlay", () => {
    const tile = render(
      <DeviceTile
        device={fixture}
        width={176}
        index={1}
        selected={false}
        onSelect={() => undefined}
        onOpen={() => undefined}
        onPrepare={() => undefined}
      />,
    );
    expect(tile.container.innerHTML).not.toContain("data:image/jpeg;base64");
    expect(tile.container.querySelector("canvas")).not.toBeNull();
    tile.unmount();

    const overlay = render(
      <FocusStream device={fixture} index={1} onClose={() => undefined} groupUdids={[]} groupMode={false} devices={[fixture]} onSelectDevice={() => undefined} />,
    );
    expect(overlay.container.innerHTML).not.toContain("data:image/jpeg;base64");
    expect(overlay.container.querySelector("canvas")).not.toBeNull();
    overlay.unmount();
  });
});
