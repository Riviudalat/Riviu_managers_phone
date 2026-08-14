import { describe, expect, it } from "vitest";
import { fittedContentRect, mapClientToImage } from "./viewHit";

describe("fittedContentRect", () => {
  it("fill uses the whole box", () => {
    expect(fittedContentRect({ width: 400, height: 832 }, 288, 600, "fill")).toEqual({
      x: 0,
      y: 0,
      width: 400,
      height: 832,
    });
  });

  it("contain letterboxes a tall image in a wider box", () => {
    const content = fittedContentRect({ width: 400, height: 400 }, 200, 400, "contain");
    expect(content.width).toBe(200);
    expect(content.height).toBe(400);
    expect(content.x).toBe(100);
    expect(content.y).toBe(0);
  });
});

describe("mapClientToImage", () => {
  const pane = { left: 100, top: 50, width: 400, height: 832 };

  it("maps the centre of a filled pane to the centre of the encoded frame", () => {
    const hit = mapClientToImage(pane, 300, 466, 288, 600, "fill");
    expect(hit).not.toBeNull();
    expect(hit!.x).toBeCloseTo(144, 5);
    expect(hit!.y).toBeCloseTo(300, 5);
  });

  it("maps a postage-stamp canvas, not the black pane around it", () => {
    // Encoded 288×600 drawn at intrinsic size, centred in a 400×832 pane.
    const canvas = { left: 156, top: 166, width: 288, height: 600 };
    const topLeft = mapClientToImage(canvas, 156, 166, 288, 600, "fill");
    expect(topLeft).toEqual({ x: 0, y: 0 });
  });

  it("the old pane-wide map of a corner click is the inaccurate tap we shipped", () => {
    const canvas = { left: 156, top: 166, width: 288, height: 600 };
    const onImage = mapClientToImage(canvas, 156, 166, 288, 600, "fill");
    const onPane = mapClientToImage(pane, 156, 166, 288, 600, "fill");
    expect(onImage).toEqual({ x: 0, y: 0 });
    expect(onPane).not.toBeNull();
    expect(onPane!.x).toBeCloseTo(40.32, 1);
    expect(onPane!.y).toBeCloseTo(83.65, 1);
  });

  it("ignores a click on the letterbox instead of clamping it onto the bezel", () => {
    const box = { left: 0, top: 0, width: 400, height: 400 };
    expect(mapClientToImage(box, 10, 200, 200, 400, "contain")).toBeNull();
    expect(mapClientToImage(box, 200, 200, 200, 400, "contain")).toEqual({ x: 100, y: 200 });
  });

  it("refuses a zero-sized frame so a tap cannot invent a coordinate", () => {
    expect(mapClientToImage(pane, 300, 466, 0, 600, "fill")).toBeNull();
  });

  it("keeps the Flow coordinate-picker mapping", () => {
    const hit = mapClientToImage(
      { left: 100, top: 50, width: 375, height: 667 },
      287.5,
      383.5,
      375,
      667,
      "contain",
    );
    expect(hit).not.toBeNull();
    expect(hit!.x).toBeCloseTo(187.5, 5);
    expect(hit!.y).toBeCloseTo(333.5, 5);
  });
});
