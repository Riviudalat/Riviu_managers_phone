import type { FlowCoordinateFrame, ImageCoordinateTarget } from "../../types";

export function projectContainedImageClick(
  frame: FlowCoordinateFrame,
  rect: DOMRect,
  clientX: number,
  clientY: number,
): ImageCoordinateTarget | null {
  if (
    frame.imageWidth <= 0 ||
    frame.imageHeight <= 0 ||
    rect.width <= 0 ||
    rect.height <= 0
  ) {
    return null;
  }
  const scale = Math.min(rect.width / frame.imageWidth, rect.height / frame.imageHeight);
  const shownWidth = frame.imageWidth * scale;
  const shownHeight = frame.imageHeight * scale;
  const left = rect.left + (rect.width - shownWidth) / 2;
  const top = rect.top + (rect.height - shownHeight) / 2;
  if (
    clientX < left ||
    clientX > left + shownWidth ||
    clientY < top ||
    clientY > top + shownHeight
  ) {
    return null;
  }
  return {
    x: (clientX - left) / scale,
    y: (clientY - top) / scale,
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
