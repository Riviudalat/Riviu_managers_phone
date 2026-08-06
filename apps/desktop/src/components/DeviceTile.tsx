import { memo, useRef, useState } from "react";
import { tileStreamStateView } from "../types";
import type { DeviceInfo, TileSize } from "../types";
import { deviceSwipe, deviceTap, groupInput, latestFrame } from "../api";
import { useDeviceFrame, useHydratedDeviceFrame } from "../frameStore";

const DEFAULT_W = 375;
const DEFAULT_H = 667;

const TILE_W: Record<TileSize, number> = {
  thumbnail: 120,
  medium: 160,
  large: 200,
  extraLarge: 248,
};

interface Props {
  device: DeviceInfo;
  tileSize: TileSize;
  selected: boolean;
  focused?: boolean;
  onSelect: (udid: string, additive: boolean) => void;
  onOpen: (udid: string) => void;
  onPrepare: (udid: string) => void;
  groupUdids: string[];
  groupMode: boolean;
}

function statusText(device: DeviceInfo) {
  if (device.status === "ready" && device.wdaReady) return "Running";
  if (device.status === "preparing" || device.status === "busy") return "Starting";
  if (device.status === "error") return "Error";
  return device.status;
}

function mapToDevice(
  img: HTMLImageElement,
  clientX: number,
  clientY: number,
): { x: number; y: number } {
  const rect = img.getBoundingClientRect();
  const nw = img.naturalWidth || DEFAULT_W;
  const nh = img.naturalHeight || DEFAULT_H;
  if (rect.width <= 0 || rect.height <= 0) {
    return { x: nw / 2, y: nh / 2 };
  }
  const x = ((clientX - rect.left) / rect.width) * nw;
  const y = ((clientY - rect.top) / rect.height) * nh;
  return {
    x: Math.max(0, Math.min(nw, x)),
    y: Math.max(0, Math.min(nh, y)),
  };
}

async function sendGesture(
  device: DeviceInfo,
  groupMode: boolean,
  groupUdids: string[],
  start: { x: number; y: number },
  end: { x: number; y: number },
  imageW: number,
  imageH: number,
) {
  const targets = groupMode && groupUdids.length > 1 ? groupUdids : [device.udid];
  const dist = Math.hypot(end.x - start.x, end.y - start.y);
  try {
    if (dist < 8) {
      if (targets.length > 1) {
        await groupInput({
          udids: targets,
          kind: "tap",
          x: end.x,
          y: end.y,
          imageW,
          imageH,
        });
      } else {
        await deviceTap(device.udid, end.x, end.y, imageW, imageH);
      }
    } else if (targets.length > 1) {
      await groupInput({
        udids: targets,
        kind: "swipe",
        x: start.x,
        y: start.y,
        toX: end.x,
        toY: end.y,
        imageW,
        imageH,
      });
    } else {
      await deviceSwipe(device.udid, start.x, start.y, end.x, end.y, imageW, imageH);
    }
  } catch (e) {
    window.alert(`Điều khiển thất bại:\n${e}`);
  }
}

function DeviceTileInner({
  device,
  tileSize,
  selected,
  focused,
  onSelect,
  onOpen,
  onPrepare,
  groupUdids,
  groupMode,
}: Props) {
  useHydratedDeviceFrame(device.udid, latestFrame);
  const frame = useDeviceFrame(device.udid);
  const imgRef = useRef<HTMLImageElement>(null);
  const drag = useRef<{ x: number; y: number } | null>(null);
  const [ratio, setRatio] = useState(DEFAULT_H / DEFAULT_W);
  const width = TILE_W[tileSize];
  const status = statusText(device);
  const streamState = tileStreamStateView(
    device.tileStreamState,
    Boolean(frame),
    Boolean(device.lastError),
  );
  const emptyLabel =
    streamState.state === "sampling" ? "Đang mở stream…" : device.lastError || "No stream";

  return (
    <article
      className={`dev-window ${selected ? "selected" : ""} ${focused ? "focused" : ""}`}
      style={{ width: width + 2 }}
    >
      <header
        className="dev-window-bar"
        onClick={(e) => onSelect(device.udid, e.metaKey || e.ctrlKey || e.shiftKey)}
        onDoubleClick={() => onOpen(device.udid)}
      >
        <label className="dev-window-check" onClick={(e) => e.stopPropagation()}>
          <input
            type="checkbox"
            checked={selected}
            onChange={() => onSelect(device.udid, true)}
          />
        </label>
        <span className="dev-window-title" title={device.name}>
          {device.name}
        </span>
        <span className={`dev-window-dot ${device.wdaReady ? "on" : ""}`} title={status} />
        <button
          type="button"
          className="dev-window-x"
          title="Phóng to"
          onClick={(e) => {
            e.stopPropagation();
            onOpen(device.udid);
          }}
        >
          ↗
        </button>
      </header>

      <div className="dev-window-screen" style={{ aspectRatio: `1 / ${ratio}` }}>
        {frame ? (
          <img
            ref={imgRef}
            src={`data:image/jpeg;base64,${frame}`}
            alt={device.name}
            draggable={false}
            className="dev-window-touch"
            title="Click / kéo để điều khiển máy"
            onLoad={(e) => {
              const el = e.currentTarget;
              if (el.naturalWidth > 0) {
                const next = el.naturalHeight / el.naturalWidth;
                setRatio((prev) => (Math.abs(prev - next) > 0.01 ? next : prev));
              }
            }}
            onPointerDown={(e) => {
              if (e.button !== 0 || !imgRef.current) return;
              e.preventDefault();
              e.stopPropagation();
              drag.current = mapToDevice(imgRef.current, e.clientX, e.clientY);
              e.currentTarget.setPointerCapture(e.pointerId);
            }}
            onPointerUp={async (e) => {
              if (e.button !== 0 || !drag.current || !imgRef.current) {
                drag.current = null;
                return;
              }
              e.preventDefault();
              e.stopPropagation();
              const start = drag.current;
              const end = mapToDevice(imgRef.current, e.clientX, e.clientY);
              drag.current = null;
              const iw = imgRef.current.naturalWidth || DEFAULT_W;
              const ih = imgRef.current.naturalHeight || DEFAULT_H;
              await sendGesture(device, groupMode, groupUdids, start, end, iw, ih);
            }}
            onPointerCancel={() => {
              drag.current = null;
            }}
          />
        ) : (
          <div className="dev-window-empty">
            <span>{emptyLabel}</span>
            <button
              type="button"
              className="link"
              onClick={(e) => {
                e.stopPropagation();
                onPrepare(device.udid);
              }}
            >
              Start
            </button>
          </div>
        )}
      </div>

      <footer className="dev-window-foot">
        <span
          className={`dev-window-stream-state is-${streamState.state}`}
          title={`Stream: ${streamState.label}; device: ${status}`}
        >
          <span aria-hidden="true" className="dev-window-stream-dot" />
          {streamState.label}
        </span>
        <span>{device.connection.toUpperCase()}</span>
        <span>
          {device.model} · {device.iosVersion}
        </span>
      </footer>
    </article>
  );
}

export const DeviceTile = memo(DeviceTileInner);
