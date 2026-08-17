import { memo } from "react";
import { deviceModelOsLabel } from "../types";
import type { DeviceInfo } from "../types";
import { streamPlaceholder } from "../streamPlaceholder";
import { useViewDecodeFailed, useViewLive, useViewSize } from "../viewStore";
import { PhoneCanvas } from "./PhoneCanvas";
import { StreamPlaceholder } from "./StreamPlaceholder";

interface Props {
  device: DeviceInfo;
  /** Tile width in px, driven by the wheel zoom. */
  width: number;
  /** 1-based position in the visible grid, shown like GenFarmer's big number. */
  index: number;
  selected: boolean;
  focused?: boolean;
  /// This phone is the one the overlay drives while Sync is on; the rest follow it.
  controlCenter?: boolean;
  onSelect: (udid: string, additive: boolean) => void;
  onOpen: (udid: string) => void;
  onPrepare: (udid: string) => void;
  /** Right-click. The tile owns no menu itself; the page places one. */
  onContextMenu?: (udid: string, x: number, y: number) => void;
}

function DeviceTileInner({
  device,
  onContextMenu,
  width,
  index,
  selected,
  focused,
  controlCenter,
  onSelect,
  onOpen,
  onPrepare,
}: Props) {
  const hasView = useViewLive(device.udid);
  const viewSize = useViewSize(device.udid);
  // Read at last. This existed the whole time and nothing consulted it, so a stream every
  // codec had already refused kept breathing the loading mark as if it were on its way.
  const decodeFailed = useViewDecodeFailed(device.udid);
  const { view } = streamPlaceholder({
    hasView,
    hasGeometry: Boolean(viewSize?.width && viewSize.width > 0),
    decodeFailed,
    tileStreamState: device.tileStreamState,
    lastError: device.lastError,
  });

  return (
    <article
      className={`dev-phone ${selected ? "selected" : ""} ${focused ? "focused" : ""}`}
      data-testid="device-tile"
      style={{ width, height: width * 2 }}
      onClick={(e) => onSelect(device.udid, e.metaKey || e.ctrlKey || e.shiftKey)}
      onContextMenu={(e) => {
        if (!onContextMenu) return;
        e.preventDefault();
        onContextMenu(device.udid, e.clientX, e.clientY);
      }}
      onDoubleClick={() => onOpen(device.udid)}
    >
      {/* Every tile keeps the same phone-shaped frame regardless of stream
          state or the stream's own aspect, so the grid never reflows when a
          frame arrives — the "fixed frame" the operator asked for. */}
      <div className="dev-phone-screen">
        <PhoneCanvas udid={device.udid} surfaceId="tile" />
        <StreamPlaceholder
          view={view}
          deviceName={device.name}
          onRetry={() => onPrepare(device.udid)}
        />

        <span className="dev-phone-conn">{device.connection.toUpperCase()}</span>

        {/* Named on the tile, because a designation nobody can see is a designation
            nobody trusts -- which is what the old implicit "first in the selection"
            master was. */}
        {controlCenter && (
          <span className="dev-phone-center" title="Trung tâm điều khiển">
            Trung tâm
          </span>
        )}

        <div className="dev-phone-info">
          <span className="dev-phone-index">{index}</span>
          <span className="dev-phone-name" title={device.name}>
            {device.name}
          </span>
          <span className="dev-phone-model">{deviceModelOsLabel(device)}</span>
        </div>
      </div>

      {/* Outside `.dev-phone-screen` on purpose. That element has `overflow: hidden`,
          so a checkbox pinned to the outer corner from inside it would be clipped —
          this sits on `.dev-phone`, which does not clip.

          There is no expand button any more: double-click on the tile opens the
          overlay, which `onDoubleClick` above has always done. */}
      <input
        className="dev-phone-pick"
        type="checkbox"
        title="Chọn máy"
        checked={selected}
        onClick={(e) => e.stopPropagation()}
        onDoubleClick={(e) => e.stopPropagation()}
        onChange={() => onSelect(device.udid, true)}
      />
    </article>
  );
}

export const DeviceTile = memo(DeviceTileInner);
