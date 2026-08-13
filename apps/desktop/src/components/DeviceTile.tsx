import { memo, useState } from "react";
import { deviceModelOsLabel, tileStreamStateView } from "../types";
import type { DeviceInfo, TileSize } from "../types";
import { latestFrame } from "../api";
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
}

function statusText(device: DeviceInfo) {
  if (device.status === "ready" && device.wdaReady) return "Running";
  if (device.status === "preparing" || device.status === "busy") return "Starting";
  if (device.status === "error") return "Error";
  return device.status;
}

function DeviceTileInner({
  device,
  tileSize,
  selected,
  focused,
  onSelect,
  onOpen,
  onPrepare,
}: Props) {
  useHydratedDeviceFrame(device.udid, latestFrame);
  const frame = useDeviceFrame(device.udid);
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
          title="Mở điều khiển"
          onClick={(e) => {
            e.stopPropagation();
            onOpen(device.udid);
          }}
        >
          ↗
        </button>
      </header>

      <div
        className="dev-window-screen"
        style={{ aspectRatio: `1 / ${ratio}` }}
        title="Bấm để mở điều khiển"
      >
        {frame ? (
          <img
            src={`data:image/jpeg;base64,${frame}`}
            alt={device.name}
            draggable={false}
            className="dev-window-touch"
            onLoad={(e) => {
              const el = e.currentTarget;
              if (el.naturalWidth > 0) {
                const next = el.naturalHeight / el.naturalWidth;
                setRatio((prev) => (Math.abs(prev - next) > 0.01 ? next : prev));
              }
            }}
            onClick={() => onOpen(device.udid)}
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
        <span>{deviceModelOsLabel(device)}</span>
      </footer>
    </article>
  );
}

export const DeviceTile = memo(DeviceTileInner);
