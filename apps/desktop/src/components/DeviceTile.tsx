import { memo } from "react";
import { deviceModelOsLabel, tileStreamStateView } from "../types";
import type { DeviceInfo } from "../types";
import { useViewLive } from "../viewStore";
import { PhoneCanvas } from "./PhoneCanvas";

interface Props {
  device: DeviceInfo;
  /** Tile width in px, driven by the wheel zoom. */
  width: number;
  /** 1-based position in the visible grid, shown like GenFarmer's big number. */
  index: number;
  selected: boolean;
  focused?: boolean;
  onSelect: (udid: string, additive: boolean) => void;
  onOpen: (udid: string) => void;
  onPrepare: (udid: string) => void;
}

function DeviceTileInner({
  device,
  width,
  index,
  selected,
  focused,
  onSelect,
  onOpen,
  onPrepare,
}: Props) {
  const hasView = useViewLive(device.udid);
  const streamState = tileStreamStateView(
    device.tileStreamState,
    hasView,
    Boolean(device.lastError),
  );
  const emptyLabel =
    streamState.state === "sampling" ? "Đang mở stream…" : device.lastError || "No stream";

  return (
    <article
      className={`dev-phone ${selected ? "selected" : ""} ${focused ? "focused" : ""}`}
      style={{ width, height: width * 2 }}
      onClick={(e) => onSelect(device.udid, e.metaKey || e.ctrlKey || e.shiftKey)}
      onDoubleClick={() => onOpen(device.udid)}
    >
      {/* Every tile keeps the same phone-shaped frame regardless of stream
          state or the stream's own aspect, so the grid never reflows when a
          frame arrives — the "fixed frame" the operator asked for. */}
      <div className="dev-phone-screen">
        <PhoneCanvas udid={device.udid} surfaceId="tile" />
        {!hasView && (
          <div className="dev-phone-empty">
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

        <span className="dev-phone-conn">{device.connection.toUpperCase()}</span>

        <div className="dev-phone-info">
          <span className="dev-phone-index-row">
            <span className="dev-phone-index">{index}</span>
            <input
              type="checkbox"
              title="Chọn máy"
              checked={selected}
              onClick={(e) => e.stopPropagation()}
              onChange={() => onSelect(device.udid, true)}
            />
          </span>
          <span className="dev-phone-name" title={device.name}>
            {device.name}
          </span>
          <span className="dev-phone-model">{deviceModelOsLabel(device)}</span>
        </div>

        <button
          type="button"
          className="dev-phone-open"
          title="Mở điều khiển"
          onClick={(e) => {
            e.stopPropagation();
            onOpen(device.udid);
          }}
        >
          ↗
        </button>
      </div>
    </article>
  );
}

export const DeviceTile = memo(DeviceTileInner);
