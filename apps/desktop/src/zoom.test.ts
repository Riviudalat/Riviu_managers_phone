import { beforeEach, describe, expect, it } from "vitest";
import {
  clampZoom,
  FOCUS_ZOOM,
  loadZoom,
  stepZoom,
  storeZoom,
  TILE_ZOOM,
  wheelWantsZoom,
} from "./zoom";

describe("wheelWantsZoom", () => {
  it("zooms only when Ctrl is held, so a plain wheel still scrolls", () => {
    expect(wheelWantsZoom({ ctrlKey: true })).toBe(true);
    expect(wheelWantsZoom({ ctrlKey: false })).toBe(false);
  });
});

describe("stepZoom", () => {
  it("zooms in on wheel up and out on wheel down", () => {
    expect(stepZoom(TILE_ZOOM, 200, -120)).toBe(220);
    expect(stepZoom(TILE_ZOOM, 220, 120)).toBe(200);
  });

  it("never leaves the range no matter how far the wheel spins", () => {
    expect(stepZoom(TILE_ZOOM, TILE_ZOOM.max, -120)).toBe(TILE_ZOOM.max);
    expect(stepZoom(TILE_ZOOM, TILE_ZOOM.min, 120)).toBe(TILE_ZOOM.min);
  });

  it("a zero delta only clamps", () => {
    expect(stepZoom(TILE_ZOOM, 5000, 0)).toBe(TILE_ZOOM.max);
  });
});

describe("clampZoom", () => {
  it("replaces a non-finite value with the fallback instead of NaN-sizing the grid", () => {
    expect(clampZoom(TILE_ZOOM, Number.NaN)).toBe(TILE_ZOOM.fallback);
    expect(clampZoom(FOCUS_ZOOM, Number.POSITIVE_INFINITY)).toBe(FOCUS_ZOOM.max);
  });
});

describe("persistence", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("round-trips through localStorage", () => {
    storeZoom(TILE_ZOOM, 240);
    expect(loadZoom(TILE_ZOOM)).toBe(240);
  });

  it("falls back when the stored value is garbage or missing", () => {
    expect(loadZoom(TILE_ZOOM)).toBe(TILE_ZOOM.fallback);
    window.localStorage.setItem(TILE_ZOOM.key, "not-a-number");
    expect(loadZoom(TILE_ZOOM)).toBe(TILE_ZOOM.fallback);
  });

  it("clamps a stored value from an older range so a huge tile cannot come back", () => {
    window.localStorage.setItem(TILE_ZOOM.key, "9999");
    expect(loadZoom(TILE_ZOOM)).toBe(TILE_ZOOM.max);
  });
});
