import { cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { PhoneCanvas } from "./PhoneCanvas";

const attach = vi.fn();
const detach = vi.fn();

vi.mock("../viewStore", () => ({
  attachViewCanvas: (...args: unknown[]) => attach(...args),
  detachViewCanvas: (...args: unknown[]) => detach(...args),
}));

beforeEach(() => {
  vi.clearAllMocks();
  // jsdom has no OffscreenCanvas, and the component's fallback path skips the attach
  // entirely — which would make every assertion below vacuous.
  (HTMLCanvasElement.prototype as unknown as { transferControlToOffscreen: () => unknown })
    .transferControlToOffscreen = function transferControlToOffscreen() {
    return { fake: "offscreen" };
  };
});

afterEach(cleanup);

describe("PhoneCanvas", () => {
  it("keeps the same canvas when only its class changes", () => {
    // `className` was a dependency of the effect that creates the canvas, and the overlay
    // passes `focus-touch is-busy` while an action runs. So every rotate, install, adb
    // call, import, export and screenshot tore the canvas down and built another -- twice
    // each, on and off -- and tearing it down closes the decoder, so the picture went black
    // until the next keyframe.
    const { container, rerender } = render(
      <PhoneCanvas udid="ce06" surfaceId="overlay" fill className="focus-touch" />,
    );
    const before = container.querySelector("canvas");
    expect(attach).toHaveBeenCalledTimes(1);

    rerender(
      <PhoneCanvas udid="ce06" surfaceId="overlay" fill className="focus-touch is-busy" />,
    );

    expect(container.querySelector("canvas")).toBe(before);
    expect(detach).not.toHaveBeenCalled();
    expect(attach).toHaveBeenCalledTimes(1);
    // And the class still followed, because that is what the prop is for.
    expect(before?.className).toBe("phone-canvas is-fill focus-touch is-busy");
  });

  it("still rebuilds when the device changes, because that is a different stream", () => {
    const { rerender } = render(<PhoneCanvas udid="ce06" surfaceId="overlay" />);
    expect(attach).toHaveBeenCalledWith("ce06", expect.anything(), "overlay");

    rerender(<PhoneCanvas udid="ce07" surfaceId="overlay" />);

    expect(detach).toHaveBeenCalledWith("ce06", "overlay");
    expect(attach).toHaveBeenCalledWith("ce07", expect.anything(), "overlay");
  });

  it("styles the canvas on the first render, not one frame later", () => {
    const { container } = render(
      <PhoneCanvas udid="ce06" surfaceId="tile" className="tile-touch" />,
    );
    expect(container.querySelector("canvas")?.className).toBe("phone-canvas tile-touch");
  });

  it("styles the canvas it builds for a different phone", () => {
    // The other side of the fix above. `className` is rightly not a dependency of
    // the effect that creates the canvas, but `udid` is — so switching phones in the
    // overlay ("Đổi máy") builds a fresh canvas while the styling effect, which depends
    // only on `className` and `fill`, stays asleep. The new canvas came up with no
    // class at all: none of `.phone-canvas`, `.is-fill` or `.focus-touch` applied,
    // so the video drew at its intrinsic bitmap size centred in the pane instead of
    // filling it, and every tap on the area around it measured against a box that was
    // no longer where the picture was.
    const { container, rerender } = render(
      <PhoneCanvas udid="ce06" surfaceId="overlay" fill className="focus-touch" />,
    );
    const before = container.querySelector("canvas");
    expect(before?.className).toBe("phone-canvas is-fill focus-touch");

    rerender(<PhoneCanvas udid="ce11" surfaceId="overlay" fill className="focus-touch" />);

    const after = container.querySelector("canvas");
    expect(after, "a different phone gets a different canvas").not.toBe(before);
    expect(after?.className).toBe("phone-canvas is-fill focus-touch");
  });
});
