import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { DeviceInfo } from "./types";
import { useBoxSelection } from "./useBoxSelection";

/**
 * The stateful half of grid selection.
 *
 * `boxSelect.ts` has always been pure and always been tested; what had no test was the part
 * that lived in `App.tsx` — the click semantics and the Ctrl/Cmd+A listener, both of which
 * are rules rather than arithmetic. Reaching them meant rendering the whole shell.
 */

const fleet = (...udids: string[]) =>
  udids.map((udid) => ({ udid, name: udid }) as unknown as DeviceInfo);

function press(key: string, target?: EventTarget) {
  const event = new KeyboardEvent("keydown", { key, ctrlKey: true, cancelable: true });
  if (target) Object.defineProperty(event, "target", { value: target });
  window.dispatchEvent(event);
  return event;
}

describe("useBoxSelection", () => {
  it("a plain click replaces the selection", () => {
    const devices = fleet("a", "b", "c");
    const { result } = renderHook(() => useBoxSelection(devices, devices, true));

    act(() => result.current.onSelect("a", false));
    act(() => result.current.onSelect("b", false));
    expect(result.current.selected).toEqual(["b"]);
  });

  it("a modified click toggles one phone without disturbing the rest", () => {
    const devices = fleet("a", "b", "c");
    const { result } = renderHook(() => useBoxSelection(devices, devices, true));

    act(() => result.current.onSelect("a", true));
    act(() => result.current.onSelect("b", true));
    act(() => result.current.onSelect("a", true));
    expect(result.current.selected).toEqual(["b"]);
  });

  it("clicking the only selected phone clears the selection", () => {
    // Otherwise there is no way back to "nothing selected" with the mouse, and on this app
    // an empty selection means *the whole fleet* to every group action.
    const devices = fleet("a", "b");
    const { result } = renderHook(() => useBoxSelection(devices, devices, true));

    act(() => result.current.onSelect("a", false));
    act(() => result.current.onSelect("a", false));
    expect(result.current.selected).toEqual([]);
  });

  it("clicking one of several selected phones narrows to it rather than clearing", () => {
    const devices = fleet("a", "b");
    const { result } = renderHook(() => useBoxSelection(devices, devices, true));

    act(() => result.current.onSelect("a", true));
    act(() => result.current.onSelect("b", true));
    act(() => result.current.onSelect("a", false));
    expect(result.current.selected).toEqual(["a"]);
  });

  it("Ctrl+A takes the visible tab, not the whole fleet", () => {
    const devices = fleet("a", "b", "c");
    const visible = fleet("a", "b");
    const { result } = renderHook(() => useBoxSelection(devices, visible, true));

    act(() => {
      press("a");
    });
    expect(result.current.selected).toEqual(["a", "b"]);
  });

  it("Ctrl+A inside a text field is left to the browser", () => {
    // Select-all while typing a group name has to select the text, not twenty phones.
    const devices = fleet("a", "b");
    const { result } = renderHook(() => useBoxSelection(devices, devices, true));
    const input = document.createElement("input");

    let event: KeyboardEvent | undefined;
    act(() => {
      event = press("a", input);
    });
    expect(result.current.selected).toEqual([]);
    expect(event?.defaultPrevented).toBe(false);
  });

  it("Ctrl+A does nothing while the grid is not on screen", () => {
    const devices = fleet("a", "b");
    const { result } = renderHook(() => useBoxSelection(devices, devices, false));

    act(() => {
      press("a");
    });
    expect(result.current.selected).toEqual([]);
  });

  it("selectedDevices follows the selection and keeps fleet order", () => {
    const devices = fleet("a", "b", "c");
    const { result } = renderHook(() => useBoxSelection(devices, devices, true));

    act(() => result.current.onSelect("c", true));
    act(() => result.current.onSelect("a", true));
    expect(result.current.selectedDevices.map((d) => d.udid)).toEqual(["a", "c"]);
  });

  it("a right-click on the canvas starts no marquee", () => {
    // The marquee is a left-drag on empty space; the right button belongs to the context menu.
    const devices = fleet("a");
    const { result } = renderHook(() => useBoxSelection(devices, devices, true));
    const node = document.createElement("div");

    act(() => {
      result.current.onCanvasMouseDown({
        button: 2,
        target: node,
        currentTarget: node,
        clientX: 10,
        clientY: 10,
        preventDefault: () => undefined,
      } as unknown as React.MouseEvent<HTMLDivElement>);
    });
    expect(result.current.band).toBeNull();
  });

  it("a mousedown that lands on a tile is that tile's click, not a marquee", () => {
    const devices = fleet("a");
    const { result } = renderHook(() => useBoxSelection(devices, devices, true));
    const canvas = document.createElement("div");
    const tile = document.createElement("article");

    act(() => {
      result.current.onCanvasMouseDown({
        button: 0,
        target: tile,
        currentTarget: canvas,
        clientX: 10,
        clientY: 10,
        preventDefault: () => undefined,
      } as unknown as React.MouseEvent<HTMLDivElement>);
    });
    expect(result.current.band).toBeNull();
  });

  it("a left-drag from empty canvas opens a marquee", () => {
    const devices = fleet("a");
    const { result } = renderHook(() => useBoxSelection(devices, devices, true));
    const canvas = document.createElement("div");

    act(() => {
      result.current.onCanvasMouseDown({
        button: 0,
        target: canvas,
        currentTarget: canvas,
        clientX: 10,
        clientY: 20,
        preventDefault: () => undefined,
      } as unknown as React.MouseEvent<HTMLDivElement>);
    });
    expect(result.current.band).not.toBeNull();
  });
});
