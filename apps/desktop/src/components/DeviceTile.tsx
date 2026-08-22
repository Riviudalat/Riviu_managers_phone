import { memo } from "react";
import { deviceTileSubtitle } from "../types";
import type { DeviceInfo } from "../types";
import { streamPlaceholder } from "../streamPlaceholder";
import { useViewDecodeFailed, useViewLive, useViewSize } from "../viewStore";
import { PhoneCanvas } from "./PhoneCanvas";
import { StreamPlaceholder } from "./StreamPlaceholder";

interface Props {
  device: DeviceInfo;
  /** Tile width in px, driven by the wheel zoom. */
  width: number;
  /**
   * The big number on the tile.
   *
   * The operator's own number for this phone when they set one, and its 1-based position in
   * the visible grid otherwise — decided by `deviceNaming.tileNumber`, not here, because the
   * fallback is a rule and not a default value.
   */
  index: number;
  /**
   * What to call this phone: the operator's alias, or the name the phone reports. Resolved
   * by `deviceNaming.tileName`. The udid is still the tooltip and the identity.
   */
  name?: string;
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
  name,
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
      data-udid={device.udid}
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
          deviceName={name ?? device.name}
          onRetry={() => onPrepare(device.udid)}
        />

        <span className="dev-phone-conn">{device.connection.toUpperCase()}</span>

        {/* Named on the tile, because a designation nobody can see is a designation
            nobody trusts -- which is what the old implicit "first in the selection"
            master was. */}
        {controlCenter && (
          <span
            className="dev-phone-center"
            title="Máy chính: khi bật Sync, mở máy nào cũng ra màn hình này và mọi máy đã chọn làm theo thao tác trên đó"
          >
            Máy chính
          </span>
        )}

        <div className="dev-phone-info">
          <span className="dev-phone-index">{index}</span>
          {/* The alias when there is one, and the phone's own name in the tooltip beside
              the udid — renaming a tile must not hide which phone it is. */}
          <span className="dev-phone-name" title={`${device.name} · ${device.udid}`}>
            {name ?? device.name}
          </span>
          <span className="dev-phone-model">{deviceTileSubtitle(device)}</span>
        </div>
      </div>

      {/* No corner checkbox and no expand button. Both were removed rather than restyled,
          because the tile already carries every gesture they duplicated: a click selects
          (Ctrl/Shift/Cmd extends), a drag across the grid box-selects, Ctrl+A takes the tab,
          and a double-click opens the overlay. A 15 px control over a live video frame that
          does what clicking the frame does is one more thing to aim at and one more thing to
          hit by accident. Selection is still visible — the tile's own border. */}
    </article>
  );
}

export const DeviceTile = memo(DeviceTileInner);
