/**
 * Rubber-band (box) selection geometry (A7, xiaowei lưới chọn kéo-khung).
 *
 * Pure and DOM-free so it is unit-testable: the React layer reads tile rects from the DOM
 * with `getBoundingClientRect` and hands them here in the same coordinate space as the drag
 * points (client coordinates work directly — a scaled/zoomed grid still reports real rendered
 * rects, so intersection stays correct without undoing the transform).
 */

export interface Rect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export interface TileRect {
  udid: string;
  rect: Rect;
}

/** Two drag corners → a normalized rect, whichever way the operator dragged. */
export function normalizeBox(x0: number, y0: number, x1: number, y1: number): Rect {
  return {
    left: Math.min(x0, x1),
    top: Math.min(y0, y1),
    right: Math.max(x0, x1),
    bottom: Math.max(y0, y1),
  };
}

/**
 * A drag only counts as a box once it clears a few pixels, so a plain click — which fires a
 * mousedown and mouseup at almost the same point — stays a click and does not wipe the
 * selection. Threshold is Chebyshev distance (max of the axes), which matches how a small
 * hand tremor reads.
 */
export function isDragMeaningful(
  x0: number,
  y0: number,
  x1: number,
  y1: number,
  threshold = 4,
): boolean {
  return Math.max(Math.abs(x1 - x0), Math.abs(y1 - y0)) >= threshold;
}

/** Do two rects overlap at all? Edge-touching alone does not count. */
export function rectsIntersect(a: Rect, b: Rect): boolean {
  return a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
}

/**
 * The udids whose tile rect intersects the drag box, in the order the tiles were given —
 * **each one once**, and never an empty one.
 *
 * Both guards are here because of a measured bug, and it was not cosmetic. The caller collects
 * rects with `querySelectorAll("[data-udid]")`, and a tile carries that attribute on more than
 * one element: the `<article>`, `PhoneCanvas`'s host div, and the `<canvas>` itself once a
 * stream attaches. So one tile produced two or three identical udids, the selection held each
 * phone twice, and the two counters disagreed exactly 2:1 (measured on the 20-phone fleet:
 * toolbar 3, sidebar 6). The counter was the visible half. The other half is that `selected`
 * feeds `group_input`'s `udids`, so **every group action would have been sent to the same phone
 * twice** — a tap twice, a key twice, a typed string twice.
 *
 * The caller now queries the tile element specifically, which is the real fix; this is the one
 * that cannot be undone by a future element gaining the attribute.
 */
export function tilesInBox(box: Rect, tiles: TileRect[]): string[] {
  const seen = new Set<string>();
  const hits: string[] = [];
  for (const tile of tiles) {
    if (!tile.udid || seen.has(tile.udid)) continue;
    if (!rectsIntersect(box, tile.rect)) continue;
    seen.add(tile.udid);
    hits.push(tile.udid);
  }
  return hits;
}

/**
 * Fold a box-select result into the existing selection.
 *
 * `additive` (Shift/Ctrl held) unions the hit set onto what was already selected; otherwise
 * the box replaces the selection. An empty box with no modifier clears — dragging over blank
 * space is how an operator deselects everything. Order follows `prev` then new hits, so the
 * primary/control phone (see App's `controlCenter`) keeps its place when it survives.
 */
export function applyBoxSelection(prev: string[], hits: string[], additive: boolean): string[] {
  if (!additive) return hits;
  const merged = [...prev];
  for (const udid of hits) if (!merged.includes(udid)) merged.push(udid);
  return merged;
}
