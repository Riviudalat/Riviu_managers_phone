import { describe, expect, it } from "vitest";

import {
  applyBoxSelection,
  isDragMeaningful,
  normalizeBox,
  rectsIntersect,
  tilesInBox,
  type Rect,
  type TileRect,
} from "./boxSelect";

const rect = (left: number, top: number, right: number, bottom: number): Rect => ({
  left,
  top,
  right,
  bottom,
});

describe("normalizeBox", () => {
  it("orders corners dragged bottom-right → top-left", () => {
    expect(normalizeBox(100, 100, 20, 30)).toEqual({ left: 20, top: 30, right: 100, bottom: 100 });
  });

  it("leaves an already top-left → bottom-right drag alone", () => {
    expect(normalizeBox(10, 10, 50, 60)).toEqual({ left: 10, top: 10, right: 50, bottom: 60 });
  });
});

describe("isDragMeaningful", () => {
  it("treats a near-stationary mousedown/up as a click, not a box", () => {
    expect(isDragMeaningful(200, 200, 202, 201)).toBe(false);
  });

  it("counts a clear drag as a box", () => {
    expect(isDragMeaningful(200, 200, 210, 200)).toBe(true);
  });

  it("honours a custom threshold", () => {
    expect(isDragMeaningful(0, 0, 6, 0, 8)).toBe(false);
    expect(isDragMeaningful(0, 0, 8, 0, 8)).toBe(true);
  });
});

describe("rectsIntersect", () => {
  it("overlapping rects intersect", () => {
    expect(rectsIntersect(rect(0, 0, 50, 50), rect(40, 40, 90, 90))).toBe(true);
  });

  it("disjoint rects do not", () => {
    expect(rectsIntersect(rect(0, 0, 50, 50), rect(60, 60, 90, 90))).toBe(false);
  });

  it("merely touching edges does not count as overlap", () => {
    expect(rectsIntersect(rect(0, 0, 50, 50), rect(50, 0, 90, 50))).toBe(false);
  });
});

describe("tilesInBox", () => {
  const tiles: TileRect[] = [
    { udid: "a", rect: rect(0, 0, 40, 40) },
    { udid: "b", rect: rect(50, 0, 90, 40) },
    { udid: "c", rect: rect(0, 50, 40, 90) },
    { udid: "d", rect: rect(50, 50, 90, 90) },
  ];

  it("returns only the tiles the box touches, in tile order", () => {
    // A box over the left column, clipping into the top row.
    expect(tilesInBox(rect(-5, -5, 45, 95), tiles)).toEqual(["a", "c"]);
  });

  it("selects everything when the box covers the grid", () => {
    expect(tilesInBox(rect(-10, -10, 100, 100), tiles)).toEqual(["a", "b", "c", "d"]);
  });

  it("returns nothing for a box in empty space", () => {
    expect(tilesInBox(rect(200, 200, 260, 260), tiles)).toEqual([]);
  });
});

describe("applyBoxSelection", () => {
  it("replaces the selection without a modifier", () => {
    expect(applyBoxSelection(["a", "b"], ["c", "d"], false)).toEqual(["c", "d"]);
  });

  it("clears when a modifier-less box hits nothing", () => {
    expect(applyBoxSelection(["a", "b"], [], false)).toEqual([]);
  });

  it("unions onto the existing selection when additive, without duplicates", () => {
    expect(applyBoxSelection(["a", "b"], ["b", "c"], true)).toEqual(["a", "b", "c"]);
  });

  it("keeps prior order so the control phone stays put", () => {
    expect(applyBoxSelection(["b", "a"], ["a", "c"], true)).toEqual(["b", "a", "c"]);
  });
});

/**
 * The bug these pin, measured on the 20-phone fleet: a box over three tiles left the toolbar
 * saying 3 and the sidebar saying 6. One tile carries `data-udid` on the article, on
 * `PhoneCanvas`'s host div and on the canvas, so the caller's `querySelectorAll` handed the
 * same phone in two or three times. The visible half was a wrong counter; the other half is
 * that `selected` feeds `group_input`, so every group action would have gone to that phone
 * twice.
 */
describe("tilesInBox never returns a phone twice", () => {
  const box: Rect = { left: 0, top: 0, right: 100, bottom: 100 };
  const inside = { left: 10, top: 10, right: 20, bottom: 20 };

  it("collapses the several elements one tile contributes into one udid", () => {
    const hits = tilesInBox(box, [
      { udid: "ce06", rect: inside },
      { udid: "ce06", rect: inside },
      { udid: "ce06", rect: inside },
      { udid: "ce07", rect: inside },
    ]);
    expect(hits).toEqual(["ce06", "ce07"]);
  });

  it("drops an element with no udid rather than selecting a phone called nothing", () => {
    const hits = tilesInBox(box, [
      { udid: "", rect: inside },
      { udid: "ce06", rect: inside },
    ]);
    expect(hits).toEqual(["ce06"]);
  });

  it("keeps the order the tiles were given", () => {
    const hits = tilesInBox(box, [
      { udid: "b", rect: inside },
      { udid: "a", rect: inside },
      { udid: "b", rect: inside },
    ]);
    expect(hits).toEqual(["b", "a"]);
  });
});
