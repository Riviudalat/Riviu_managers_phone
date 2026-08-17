import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DeviceInfo } from "../types";
import {
  deviceShell,
  deviceSwipe,
  deviceSwipePath,
  deviceTap,
  deviceTypeText,
  groupInput,
} from "../api";
import { FocusStream } from "./FocusStream";
import { ToastHost } from "./ToastHost";
import { resetToasts } from "../toastStore";

vi.mock("../api", () => ({
  backupDevice: vi.fn(),
  deviceControlBegin: vi.fn(async () => undefined),
  deviceControlEnd: vi.fn(async () => undefined),
  deviceKey: vi.fn(),
  deviceShell: vi.fn(async () => ({ exitCode: 0, stdout: "", stderr: "" })),
  deviceSwipePath: vi.fn(async () => undefined),
  deviceSwipe: vi.fn(async () => undefined),
  deviceTap: vi.fn(async () => undefined),
  deviceTypeText: vi.fn(async () => undefined),
  exportMedia: vi.fn(async () => 0),
  importMedia: vi.fn(async () => ""),
  groupInput: vi.fn(async () => ({ completedUdids: [], skipped: [] })),
  installIpa: vi.fn(async () => undefined),
  rebootDevice: vi.fn(),
  restoreDevice: vi.fn(),
  saveViewSnapshot: vi.fn(),
  screenshot: vi.fn(),
  setScreenRotation: vi.fn(async () => 0),
  viewRequestKeyframe: vi.fn(async () => true),
}));

// Flipped by the "no picture yet" block below. A phone that has never painted has neither a
// frame nor an encoded size, and the pair has to move together — a size with no frame is a
// state the store cannot produce.
let dark = false;
let decodeRefused = false;

vi.mock("../viewStore", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../viewStore")>();
  return {
    ...actual,
    useViewLive: () => !dark,
    useViewSize: () => (dark ? undefined : { width: 288, height: 600, generation: 1 }),
    useViewDecodeFailed: () => decodeRefused,
  };
});

// `render` binds its queries to document.body, not to the container it returns, so without
// this every test after the first searches the leftovers of the ones before it.
afterEach(cleanup);

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
      <FocusStream device={fixture} index={2} onClose={() => undefined} groupUdids={[]} groupMode={false} devices={[fixture]} onSelectDevice={() => undefined} />,
    );
    const screen = container.querySelector("[data-testid='focus-screen']");
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
      <FocusStream device={fixture} index={2} onClose={() => undefined} groupUdids={[]} groupMode={false} devices={[fixture]} onSelectDevice={() => undefined} />,
    );
    const screen = container.querySelector("[data-testid='focus-screen']");
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
      <FocusStream device={fixture} index={2} onClose={() => undefined} groupUdids={[]} groupMode={false} devices={[fixture]} onSelectDevice={() => undefined} />,
    );
    const screen = container.querySelector("[data-testid='focus-screen']");
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
      <FocusStream device={fixture} index={2} onClose={() => undefined} groupUdids={[]} groupMode={false} devices={[fixture]} onSelectDevice={() => undefined} />,
    );
    const screen = container.querySelector("[data-testid='focus-screen']");
    const canvas = container.querySelector("canvas");
    mockRect(canvas!, { left: 156, top: 166, width: 288, height: 600 });

    fireEvent.pointerDown(screen!, { button: 0, clientX: 10, clientY: 200, pointerId: 1 });
    fireEvent.pointerUp(screen!, { button: 0, clientX: 10, clientY: 200, pointerId: 1 });

    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(deviceTap).not.toHaveBeenCalled();
  });
});

describe("overlay panel rows", () => {
  const other: DeviceInfo = { ...fixture, udid: "ce07", name: "S8+" };

  beforeEach(() => {
    localStorage.clear();
    vi.mocked(deviceShell).mockClear();
    vi.mocked(deviceTypeText).mockClear();
  });

  it("switches the overlay to another phone through the parent, not a local copy", async () => {
    // Lifting the swap to `setFocusUdid` is what releases the old phone's control lease and
    // claims the new one: both the overlay preset effect and the lease effect are keyed on
    // the udid, so a locally swapped device would leave the old lease open.
    const onSelectDevice = vi.fn();
    const { getByText, findByTitle } = render(
      <FocusStream
        device={fixture}
        index={1}
        onClose={() => undefined}
        groupUdids={[]}
        groupMode={false}
        devices={[fixture, other]}
        onSelectDevice={onSelectDevice}
      />,
    );

    fireEvent.click(getByText("Đổi máy"));
    fireEvent.click(await findByTitle("ce07"));

    expect(onSelectDevice).toHaveBeenCalledWith("ce07");
  });

  it("shows the battery it was given, and a dash when there is none", () => {
    const { getByTitle, unmount } = render(
      <FocusStream
        device={{ ...fixture, battery: 58 }}
        index={1}
        onClose={() => undefined}
        groupUdids={[]}
        groupMode={false}
        devices={[fixture]}
        onSelectDevice={() => undefined}
      />,
    );
    expect(getByTitle("Pin 58%").textContent).toContain("58%");
    unmount();

    // Never a fabricated 0% or 100%: the driver returns None for a phone it could not read.
    const absent = render(
      <FocusStream
        device={fixture}
        index={1}
        onClose={() => undefined}
        groupUdids={[]}
        groupMode={false}
        devices={[fixture]}
        onSelectDevice={() => undefined}
      />,
    );
    expect(absent.getByTitle("Chưa đọc được mức pin").textContent).toContain("—");
  });

  it("types a saved phrase onto every phone the overlay is driving", async () => {
    // The group path, because that is the case the feature exists for: the same bio onto
    // twenty phones. It goes through `group_input` `type`, which reaches ACTION_SET_TEXT --
    // the only route here that carries Vietnamese diacritics.
    const { getByText, getByPlaceholderText } = render(
      <FocusStream
        device={fixture}
        index={1}
        onClose={() => undefined}
        groupUdids={["ce06", "ce07"]}
        groupMode
        devices={[fixture, other]}
        onSelectDevice={() => undefined}
      />,
    );

    fireEvent.click(getByText("Câu nhanh"));
    fireEvent.change(getByPlaceholderText("Nội dung (vd: xin chào)"), {
      target: { value: "xin chào các bạn" },
    });
    fireEvent.click(getByText("Lưu câu"));
    fireEvent.click(getByText("xin chào các bạn"));

    await waitFor(() =>
      expect(vi.mocked(groupInput)).toHaveBeenCalledWith({
        udids: ["ce06", "ce07"],
        kind: "type",
        text: "xin chào các bạn",
      }),
    );
  });

  it("offers only keyboards the phone itself listed, never the helper IME", async () => {
    vi.mocked(deviceShell).mockImplementation(async (_udid: string, script: string) => ({
      exitCode: 0,
      stdout: script.startsWith("ime list")
        ? "com.riviu.agent/.RiviuIme\ncom.sec.android.inputmethod/.SamsungKeypad\n"
        : "com.sec.android.inputmethod/.SamsungKeypad\n",
      stderr: "",
    }));
    const { getByText, findByText, queryByText } = render(
      <FocusStream
        device={fixture}
        index={1}
        onClose={() => undefined}
        groupUdids={[]}
        groupMode={false}
        devices={[fixture]}
        onSelectDevice={() => undefined}
      />,
    );

    fireEvent.click(getByText("Đổi bàn phím"));
    await findByText(/SamsungKeypad/);
    // Leaving the helper IME as the phone's keyboard is ruled out, so it must not be
    // offerable in the first place.
    expect(queryByText(/RiviuIme/)).toBeNull();

    fireEvent.click(await findByText(/SamsungKeypad/));
    await waitFor(() =>
      expect(vi.mocked(deviceShell)).toHaveBeenCalledWith(
        "ce06",
        "ime set com.sec.android.inputmethod/.SamsungKeypad",
      ),
    );
  });
});

describe("FocusStream with no picture yet", () => {
  beforeEach(() => {
    vi.mocked(deviceTap).mockClear();
    HTMLElement.prototype.setPointerCapture = vi.fn();
    HTMLElement.prototype.releasePointerCapture = vi.fn();
    // A phone that has never painted: no frame, and therefore no encoded size to map a
    // gesture through. This is the state the operator's screenshot showed.
    dark = true;
  });
  afterEach(() => {
    dark = false;
  });

  function renderDark(device: DeviceInfo = fixture) {
    resetToasts();
    // `ToastHost` is mounted alongside on purpose: a toast pushed into the store that never
    // reaches the screen would satisfy a store-level assertion and still leave the operator
    // staring at nothing, which is the exact failure being fixed.
    return render(
      <>
        <FocusStream
          device={device}
          index={3}
          onClose={() => undefined}
          groupUdids={[]}
          groupMode={false}
          devices={[device]}
          onSelectDevice={() => undefined}
        />
        <ToastHost />
      </>,
    );
  }

  it("shows the loading mark instead of the old flat string", () => {
    const { getByRole, queryByText } = renderDark();
    expect(getByRole("status")).toBeTruthy();
    expect(queryByText("Đang chờ stream…")).toBeNull();
  });

  it("says a gesture could not be sent rather than discarding it in silence", async () => {
    // The whole complaint: while this state was up, every pointer handler early-returned on
    // the missing frame size, so clicking the picture produced no tap, no toast and no log.
    const { container, findByText } = renderDark();
    const pane = container.querySelector("[data-testid='focus-screen']");
    expect(pane).not.toBeNull();
    mockRect(pane!, { left: 0, top: 0, width: 288, height: 600 });

    fireEvent.pointerDown(pane!, { button: 0, clientX: 100, clientY: 200, pointerId: 1 });
    fireEvent.pointerUp(pane!, { button: 0, clientX: 100, clientY: 200, pointerId: 1 });

    expect(await findByText(/chưa gửi được thao tác/i)).toBeTruthy();
    expect(vi.mocked(deviceTap)).not.toHaveBeenCalled();
  });

  it("offers a retry for a real failure and none for a codec refusal", () => {
    const failed = { ...fixture, lastError: "scrcpy-server exited" };
    const { getByRole, unmount } = renderDark(failed);
    expect(getByRole("button", { name: "Thử lại" })).toBeTruthy();
    unmount();

    // Every codec candidate was refused, so retrying the same stream cannot help and the
    // button must not be offered at all.
    decodeRefused = true;
    const { queryByRole, getByText } = renderDark();
    expect(queryByRole("button", { name: "Thử lại" })).toBeNull();
    expect(getByText(/không đọc được luồng này/i)).toBeTruthy();
    decodeRefused = false;
  });
});
