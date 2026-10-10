/**
 * Mouse-wheel zoom for the phone previews (grid tiles and the control overlay).
 *
 * One multiplicative step per wheel notch, clamped to the range. Multiplicative
 * rather than additive so a notch feels the same at 120 px as at 400 px.
 */
export interface ZoomRange {
  min: number;
  max: number;
  fallback: number;
  /** localStorage key so the chosen size survives an app restart. */
  key: string;
}

export const TILE_ZOOM: ZoomRange = {
  min: 100,
  max: 420,
  fallback: 180,
  key: "riviu.tile.width",
};

/**
 * The three named sizes on the "Hiển thị" rail. Each one only sets a starting width for the
 * same slider and Ctrl+wheel. The strip style follows the width band rather than the button
 * that was pressed, so a width picked by hand still lands in exactly one band.
 */
export type TileDensity = "overview" | "medium" | "large";

export const TILE_PRESETS: readonly { density: TileDensity; label: string; width: number }[] = [
  { density: "overview", label: "Tổng quan", width: 100 },
  { density: "medium", label: "Vừa", width: 140 },
  { density: "large", label: "Lớn", width: 190 },
];

export function tileDensity(width: number): TileDensity {
  if (width < 140) return "overview";
  if (width < 190) return "medium";
  return "large";
}

export const FOCUS_ZOOM: ZoomRange = {
  min: 260,
  max: 760,
  fallback: 400,
  key: "riviu.focus.width",
};

const STEP = 1.1;

export function clampZoom(range: ZoomRange, value: number): number {
  if (Number.isNaN(value)) return range.fallback;
  return Math.round(Math.min(range.max, Math.max(range.min, value)));
}

/** Zoom only on Ctrl+wheel. Plain wheel must keep scrolling the page. */
export function wheelWantsZoom(event: { ctrlKey: boolean }): boolean {
  return event.ctrlKey;
}

/** Wheel up (negative deltaY) zooms in; wheel down zooms out. */
export function stepZoom(range: ZoomRange, width: number, deltaY: number): number {
  if (deltaY === 0) return clampZoom(range, width);
  const factor = deltaY < 0 ? STEP : 1 / STEP;
  return clampZoom(range, width * factor);
}

export function loadZoom(range: ZoomRange): number {
  try {
    const raw = window.localStorage.getItem(range.key);
    if (raw === null) return range.fallback;
    return clampZoom(range, Number(raw));
  } catch {
    return range.fallback;
  }
}

export function storeZoom(range: ZoomRange, value: number): void {
  try {
    window.localStorage.setItem(range.key, String(clampZoom(range, value)));
  } catch {
    // Persistence is a convenience; losing it must not break zooming.
  }
}
