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
  /// Whether the operator has to do something, as opposed to just wait.
  ///
  /// Everything that is not an outright failure is on its way: `parked` is the *default*
  /// state a device is listed with, not a decision to leave it stopped, and the keeper
  /// starts a producer for every device it sees. Showing "No stream" and a Start button
  /// during that told the operator their phone was idle and needed a nudge, which was never
  /// true -- and before the leftover sweep was fixed it said so for the better part of
  /// twenty seconds. A failure is the one case where nothing is coming without them.
  const failed = streamState.state === "error" || Boolean(device.lastError);

  return (
    <article
      className={`dev-phone ${selected ? "selected" : ""} ${focused ? "focused" : ""}`}
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
        {!hasView &&
          (failed ? (
            <div className="dev-phone-empty">
              <span>{device.lastError || "Không mở được stream"}</span>
              <button
                type="button"
                className="link"
                onClick={(e) => {
                  e.stopPropagation();
                  onPrepare(device.udid);
                }}
              >
                Thử lại
              </button>
            </div>
          ) : (
            <div
              className="dev-phone-loading"
              role="status"
              aria-label={`Đang mở stream ${device.name}`}
              title="Đang mở stream…"
            >
              <img src="/logo.jpg" alt="" />
            </div>
          ))}

        <span className="dev-phone-conn">{device.connection.toUpperCase()}</span>

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
