import type { FlowCoordinateFrame, ImageCoordinateTarget } from "../../types";
import { mapClientToImage } from "../../viewHit";

/**
 * Turn a click on a letterboxed preview into a point in image space.
 *
 * Lives apart from the picker because the vision capture needs the same projection, and a
 * pure function exported from a component file costs that file its Fast Refresh.
 */
export function projectContainedImageClick(
  frame: FlowCoordinateFrame,
  rect: DOMRect,
  clientX: number,
  clientY: number,
): ImageCoordinateTarget | null {
  const hit = mapClientToImage(
    { left: rect.left, top: rect.top, width: rect.width, height: rect.height },
    clientX,
    clientY,
    frame.imageWidth,
    frame.imageHeight,
    "contain",
  );
  if (!hit) return null;
  return {
    x: hit.x,
    y: hit.y,
    imageWidth: frame.imageWidth,
    imageHeight: frame.imageHeight,
    orientation: frame.orientation,
    profileId: frame.profileId,
  };
}
