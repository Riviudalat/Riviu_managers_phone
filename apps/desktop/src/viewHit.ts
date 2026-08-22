/**
 * Map a pointer onto the pixels of a phone preview.
 *
 * The overlay used to map the whole black pane. Scrcpy encodes at
 * `max_size=600`, so the canvas bitmap is ~288×600 and CSS `width:auto`
 * drew that postage-stamp in the centre. Clicks on the image then scaled
 * against the large pane and landed in the wrong place on the phone.
 *
 * Always pass the **canvas** (or the actually painted element) rect, not
 * the letterbox around it. Clicks on the letterbox return null — do not
 * clamp them onto the bezel.
 */
export type ViewFit = "contain" | "fill";

export interface ViewBox {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface ViewHit {
  x: number;
  y: number;
}

export function fittedContentRect(
  box: Pick<ViewBox, "width" | "height">,
  imageW: number,
  imageH: number,
  fit: ViewFit,
): { x: number; y: number; width: number; height: number } {
  if (imageW <= 0 || imageH <= 0 || box.width <= 0 || box.height <= 0) {
    return { x: 0, y: 0, width: 0, height: 0 };
  }
  if (fit === "fill") {
    return { x: 0, y: 0, width: box.width, height: box.height };
  }
  const scale = Math.min(box.width / imageW, box.height / imageH);
  const width = imageW * scale;
  const height = imageH * scale;
  return {
    x: (box.width - width) / 2,
    y: (box.height - height) / 2,
    width,
    height,
  };
}

export function mapClientToImage(
  box: ViewBox,
  clientX: number,
  clientY: number,
  imageW: number,
  imageH: number,
  fit: ViewFit = "fill",
): ViewHit | null {
  const content = fittedContentRect(box, imageW, imageH, fit);
  if (content.width <= 0 || content.height <= 0) return null;
  const x = clientX - box.left - content.x;
  const y = clientY - box.top - content.y;
  if (x < 0 || y < 0 || x >= content.width || y >= content.height) {
    return null;
  }
  return {
    x: (x / content.width) * imageW,
    y: (y / content.height) * imageH,
  };
}

/** Overlay / tile: the painted <canvas>, not the black pane around it. */
export function paintedViewBox(root: ParentNode): ViewBox | null {
  const canvas = root.querySelector("canvas");
  if (!canvas) return null;
  const rect = canvas.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0) return null;
  return { left: rect.left, top: rect.top, width: rect.width, height: rect.height };
}
