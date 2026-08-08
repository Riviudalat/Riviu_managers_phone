import { useState } from "react";

import type { FlowCoordinateFrame, VisionRegion } from "../../types";
import { projectContainedImageClick } from "./FlowCoordinatePicker";

/** Crop a sub-rectangle of a base64 JPEG frame into a base64 PNG (no data-URL prefix). */
function cropToPngBase64(
  jpegBase64: string,
  x: number,
  y: number,
  width: number,
  height: number,
): Promise<string> {
  return new Promise((resolve, reject) => {
    const source = new Image();
    source.onload = () => {
      const canvas = document.createElement("canvas");
      canvas.width = width;
      canvas.height = height;
      const context = canvas.getContext("2d");
      if (!context) {
        reject(new Error("canvas 2d context unavailable"));
        return;
      }
      context.drawImage(source, x, y, width, height, 0, 0, width, height);
      const dataUrl = canvas.toDataURL("image/png");
      const base64 = dataUrl.split(",")[1] ?? "";
      resolve(base64);
    };
    source.onerror = () => reject(new Error("failed to decode device frame"));
    source.src = `data:image/jpeg;base64,${jpegBase64}`;
  });
}

/**
 * Two-click crop over a device frame: click the top-left, then the bottom-right
 * of the template region. Produces the cropped PNG (base64) plus the region in
 * screen fractions so the vision node can search only that area.
 */
export function FlowVisionCapture({
  frame,
  onCapture,
  onCancel,
}: {
  frame: FlowCoordinateFrame;
  onCapture: (templatePngBase64: string, region: VisionRegion) => void;
  onCancel: () => void;
}) {
  const [first, setFirst] = useState<{ x: number; y: number } | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handleClick = (event: React.MouseEvent<HTMLImageElement>) => {
    const point = projectContainedImageClick(
      frame,
      event.currentTarget.getBoundingClientRect(),
      event.clientX,
      event.clientY,
    );
    if (!point) return;
    if (first === null) {
      setFirst({ x: point.x, y: point.y });
      return;
    }
    const x0 = Math.round(Math.min(first.x, point.x));
    const y0 = Math.round(Math.min(first.y, point.y));
    const x1 = Math.round(Math.max(first.x, point.x));
    const y1 = Math.round(Math.max(first.y, point.y));
    const width = x1 - x0;
    const height = y1 - y0;
    if (width < 4 || height < 4) {
      setFirst(null);
      setError("Vùng chọn quá nhỏ — chọn lại hai góc.");
      return;
    }
    void cropToPngBase64(frame.jpegBase64, x0, y0, width, height)
      .then((base64) => {
        onCapture(base64, {
          x0: x0 / frame.imageWidth,
          y0: y0 / frame.imageHeight,
          x1: x1 / frame.imageWidth,
          y1: y1 / frame.imageHeight,
        });
      })
      .catch((cropError: unknown) =>
        setError(cropError instanceof Error ? cropError.message : String(cropError)),
      );
  };

  return (
    <div className="flow-vision-capture">
      <img
        src={`data:image/jpeg;base64,${frame.jpegBase64}`}
        alt="Device frame"
        draggable={false}
        onClick={handleClick}
      />
      <output>
        {first === null
          ? "Bấm góc trên-trái của mẫu"
          : "Bấm góc dưới-phải của mẫu"}{" "}
        ({frame.imageWidth} x {frame.imageHeight})
      </output>
      {error && <p role="alert">{error}</p>}
      <button type="button" onClick={onCancel}>
        Hủy
      </button>
    </div>
  );
}
