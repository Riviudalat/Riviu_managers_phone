import type { FlowCoordinateFrame, ImageCoordinateTarget } from "../../types";
import { mapClientToImage } from "../../viewHit";

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

export function FlowCoordinatePicker({
  frame,
  onPick,
}: {
  frame: FlowCoordinateFrame;
  onPick: (point: ImageCoordinateTarget) => void;
}) {
  return (
    <div className="flow-coordinate-picker">
      <img
        src={`data:image/jpeg;base64,${frame.jpegBase64}`}
        alt="Device frame"
        draggable={false}
        onClick={(event) => {
          const point = projectContainedImageClick(
            frame,
            event.currentTarget.getBoundingClientRect(),
            event.clientX,
            event.clientY,
          );
          if (point) onPick(point);
        }}
      />
      <output>
        {frame.imageWidth} x {frame.imageHeight} / {frame.orientation}
      </output>
    </div>
  );
}
