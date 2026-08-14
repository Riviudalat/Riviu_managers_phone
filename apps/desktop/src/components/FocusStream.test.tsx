import { fireEvent, render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DeviceInfo } from "../types";
import { deviceTap } from "../api";
import { FocusStream } from "./FocusStream";

vi.mock("../api", () => ({
  backupDevice: vi.fn(),
  deviceControlBegin: vi.fn(async () => undefined),
  deviceControlEnd: vi.fn(async () => undefined),
  deviceKey: vi.fn(),
  deviceSwipe: vi.fn(),
  deviceTap: vi.fn(async () => undefined),
  groupInput: vi.fn(),
  rebootDevice: vi.fn(),
  restoreDevice: vi.fn(),
  saveViewSnapshot: vi.fn(),
  screenshot: vi.fn(),
}));

vi.mock("../viewStore", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../viewStore")>();
  return {
    ...actual,
    useViewLive: () => true,
    useViewSize: () => ({ width: 288, height: 600, generation: 1 }),
  };
});

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

function mockRect(el: Element, box: { left: number; top: number; width: number; height: number }) {
  vi.spyOn(el, "getBoundingClientRect").mockReturnValue({
    x: box.left,
    y: box.top,
    left: box.left,
    top: box.top,
    width: box.width,
    height: box.height,
    right: box.left + box.width,
    bottom: box.top + box.height,
    toJSON: () => ({}),
  } as DOMRect);
}

describe("FocusStream hit mapping", () => {
  beforeEach(() => {
    vi.mocked(deviceTap).mockClear();
    HTMLElement.prototype.setPointerCapture = vi.fn();
    HTMLElement.prototype.releasePointerCapture = vi.fn();
  });

  it("taps through the painted canvas, not the black pane", async () => {
    const { container } = render(
      <FocusStream device={fixture} index={2} onClose={() => undefined} groupUdids={[]} groupMode={false} />,
    );
    const screen = container.querySelector(".focus-phone-screen");
    const canvas = container.querySelector("canvas");
    expect(screen).not.toBeNull();
    expect(canvas).not.toBeNull();
    mockRect(canvas!, { left: 0, top: 0, width: 400, height: 832 });

    fireEvent.pointerDown(screen!, { button: 0, clientX: 200, clientY: 416, pointerId: 1 });
    fireEvent.pointerUp(screen!, { button: 0, clientX: 200, clientY: 416, pointerId: 1 });

    await waitFor(() => {
      expect(deviceTap).toHaveBeenCalledWith("ce06", 144, 300, 288, 600);
    });
  });

  it("ignores a click on the letterbox so it cannot become a bezel tap", async () => {
    const { container } = render(
      <FocusStream device={fixture} index={2} onClose={() => undefined} groupUdids={[]} groupMode={false} />,
    );
    const screen = container.querySelector(".focus-phone-screen");
    const canvas = container.querySelector("canvas");
    mockRect(canvas!, { left: 156, top: 166, width: 288, height: 600 });

    fireEvent.pointerDown(screen!, { button: 0, clientX: 10, clientY: 200, pointerId: 1 });
    fireEvent.pointerUp(screen!, { button: 0, clientX: 10, clientY: 200, pointerId: 1 });

    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(deviceTap).not.toHaveBeenCalled();
  });
});
