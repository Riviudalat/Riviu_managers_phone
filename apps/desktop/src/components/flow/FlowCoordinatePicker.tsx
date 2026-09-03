import type { FlowCoordinateFrame, ImageCoordinateTarget } from "../../types";
import { SCREEN_ORIENTATION_LABELS } from "./actionPresentation";
import { projectContainedImageClick } from "./projectImageClick";

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
        alt="Khung hình thiết bị"
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
        {frame.imageWidth} x {frame.imageHeight} / {SCREEN_ORIENTATION_LABELS[frame.orientation]}
      </output>
    </div>
  );
}
