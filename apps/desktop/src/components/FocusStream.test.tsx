import { fireEvent, render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DeviceInfo } from "../types";
import { deviceSwipe, deviceSwipePath, deviceTap } from "../api";
import { FocusStream } from "./FocusStream";

vi.mock("../api", () => ({
  backupDevice: vi.fn(),
  deviceControlBegin: vi.fn(async () => undefined),
  deviceControlEnd: vi.fn(async () => undefined),
  deviceKey: vi.fn(),
  deviceSwipePath: vi.fn(async () => undefined),
  deviceSwipe: vi.fn(async () => undefined),
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
    vi.mocked(deviceSwipe).mockClear();
    vi.mocked(deviceSwipePath).mockClear();
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

  it("sends the path the pointer took, not the two endpoints", async () => {
    // The defect this fixes: the gesture used to be decided at release from `start` and
    // `end` alone, so a curved, accelerating drag reached the phone as a straight line at
    // constant speed. Every intermediate sample was discarded before it left the browser.
    const { container } = render(
      <FocusStream device={fixture} index={2} onClose={() => undefined} groupUdids={[]} groupMode={false} />,
    );
    const screen = container.querySelector(".focus-phone-screen");
    const canvas = container.querySelector("canvas");
    mockRect(canvas!, { left: 0, top: 0, width: 400, height: 832 });

    fireEvent.pointerDown(screen!, { button: 0, clientX: 100, clientY: 100, pointerId: 1 });
    // Spaced past the 8 ms sampling floor, and far enough apart to clear the 2 px one.
    for (const [x, y] of [[140, 160], [190, 260], [240, 400]]) {
      await new Promise((resolve) => setTimeout(resolve, 12));
      fireEvent.pointerMove(screen!, { clientX: x, clientY: y, pointerId: 1 });
    }
    await new Promise((resolve) => setTimeout(resolve, 12));
    fireEvent.pointerUp(screen!, { button: 0, clientX: 300, clientY: 520, pointerId: 1 });

    await waitFor(() => {
      expect(deviceSwipePath).toHaveBeenCalled();
    });
    const [udid, start, steps, imageW, imageH] = (deviceSwipePath as unknown as {
      mock: { calls: unknown[][] };
    }).mock.calls[0] as [string, { x: number; y: number }, { x: number; y: number; durationMs: number }[], number, number];
    expect(udid).toBe("ce06");
    expect(imageW).toBe(288);
    expect(imageH).toBe(600);
    // Every intermediate sample survived, plus the release point.
    expect(steps.length).toBeGreaterThanOrEqual(4);
    // The gesture must end exactly where the operator let go, whatever the sampling did.
    expect(steps.at(-1)!.x).toBeCloseTo((300 / 400) * 288, 5);
    expect(steps.at(-1)!.y).toBeCloseTo((520 / 832) * 600, 5);
    expect(start.x).toBeCloseTo((100 / 400) * 288, 5);
    // Each step carries its own duration -- that is what makes the velocity real.
    expect(steps.every((step) => step.durationMs > 0)).toBe(true);
  });

  it("falls back to two endpoints when the pointer reported too little to be a path", async () => {
    // A flick the browser only sampled once is not a curve, and pretending otherwise would
    // send a one-step path whose duration is the whole gesture.
    const { container } = render(
      <FocusStream device={fixture} index={2} onClose={() => undefined} groupUdids={[]} groupMode={false} />,
    );
    const screen = container.querySelector(".focus-phone-screen");
    const canvas = container.querySelector("canvas");
    mockRect(canvas!, { left: 0, top: 0, width: 400, height: 832 });

    fireEvent.pointerDown(screen!, { button: 0, clientX: 100, clientY: 100, pointerId: 1 });
    fireEvent.pointerUp(screen!, { button: 0, clientX: 300, clientY: 520, pointerId: 1 });

    await waitFor(() => {
      expect(deviceSwipe).toHaveBeenCalled();
    });
    expect(deviceSwipePath).not.toHaveBeenCalled();
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
