import { memo } from "react";
import { Maximize2 } from "lucide-react";
import { deviceOperationalView } from "../deviceWork";
import type { DeviceOperationalView } from "../deviceWork";
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
   * by `deviceNaming.tileName`. Technical identity stays in the details drawer.
   */
  name?: string;
  /** Shared operator-facing status; omitted only by isolated legacy/test callers. */
  operational?: DeviceOperationalView;
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
  operational: providedOperational,
  selected,
  focused,
  controlCenter,
  onSelect,
  onOpen,
  onPrepare,
}: Props) {
  const operational = providedOperational ?? deviceOperationalView(device, null);
  const displayName = name ?? device.name;
  const operationalLabel = operational.ownerLabel
    ? `${operational.label} · ${operational.ownerLabel}`
    : operational.label;
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
      role="group"
      tabIndex={0}
      aria-roledescription="thẻ thiết bị"
      aria-label={`Máy ${index}, ${displayName}, ${operationalLabel}${selected ? ", đã chọn" : ""}`}
      style={{ width, height: width * 2 }}
      onClick={(e) => onSelect(device.udid, e.metaKey || e.ctrlKey || e.shiftKey)}
      onKeyDown={(event) => {
        if (event.target !== event.currentTarget) return;
        if (event.key !== "Enter" && event.key !== " ") return;
        event.preventDefault();
        onSelect(device.udid, event.metaKey || event.ctrlKey || event.shiftKey);
      }}
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
          deviceName={displayName}
          onRetry={() => onPrepare(device.udid)}
        />

        <button
          type="button"
          className="dev-phone-open"
          aria-label={`Mở màn hình Máy ${index}`}
          title="Mở màn hình"
          onClick={(event) => {
            event.stopPropagation();
            onOpen(device.udid);
          }}
          onDoubleClick={(event) => event.stopPropagation()}
        >
          <Maximize2 size={14} aria-hidden="true" />
        </button>

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
          <span className="dev-phone-index">Máy {index}</span>
          <span className="dev-phone-name" title={displayName}>
            {displayName}
          </span>
          <span className={`dev-phone-status is-${operational.kind}`}>
            {operationalLabel}
          </span>
        </div>
      </div>

      {/* Selection stays on the tile itself. The small open action is intentionally separate:
          double-click remains efficient for pointer users, while keyboard and touch users get
          one unambiguous, named control for opening the focused stream. */}
    </article>
  );
}

export const DeviceTile = memo(DeviceTileInner);
