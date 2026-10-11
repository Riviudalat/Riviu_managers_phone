import { machineNumberLabel } from "../deviceNaming";
import { memo } from "react";
import { Check, Maximize2 } from "lucide-react";
import { deviceOperationalView, deviceStatusText } from "../deviceWork";
import type { DeviceOperationalView } from "../deviceWork";
import type { DeviceInfo } from "../types";
import { streamPlaceholder } from "../streamPlaceholder";
import { useViewDecodeFailed, useViewLive, useViewSize } from "../viewStore";
import { useFrameFreshness } from "../frameFreshness";
import { PhoneCanvas } from "./PhoneCanvas";
import { StreamPlaceholder } from "./StreamPlaceholder";
import { DeviceStatusLine } from "./DeviceStatusLine";

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
  handle?: string;
  /** Shared operator-facing status; omitted only by isolated legacy/test callers. */
  operational?: DeviceOperationalView;
  selected: boolean;
  focused?: boolean;
  /// This phone is the one the overlay drives while Sync is on; the rest follow it.
  controlCenter?: boolean;
  onSelect: (udid: string, additive: boolean) => void;
  onOpen: (udid: string) => void;
  onPrepare: (udid: string) => void;
  onHelperMaintenance?: (udid: string) => void;
  /** Right-click. The tile owns no menu itself; the page places one. */
  onContextMenu?: (udid: string, x: number, y: number) => void;
}

function DeviceTileInner({
  device,
  onContextMenu,
  width,
  index,
  name,
  handle,
  operational: providedOperational,
  selected,
  focused,
  controlCenter,
  onSelect,
  onOpen,
  onPrepare,
  onHelperMaintenance,
}: Props) {
  const operational = providedOperational ?? deviceOperationalView(device, null);
  const displayName = name ?? device.name;
  const username = handle?.trim().replace(/^@+/, "");
  const operationalLabel = deviceStatusText(operational);
  // The alias usually is the model when the operator has not named the phone, so the model is
  // only worth a second mention when it says something the name does not.
  const model = device.model?.trim();
  const shownModel = model && model !== displayName ? model : undefined;
  const identity = [username ? `@${username}` : "", shownModel ?? ""].filter(Boolean).join(" · ");
  // A phone that needs the operator gets a ring and its reason on the strip, and so does a lost
  // phone a job still holds. The reason a healthy phone carries (when control gets checked)
  // stays in the tooltip.
  const attention = operational.kind === "warning";
  const heldOffline = operational.kind === "offline" && operational.axes.work.state === "running";
  const detail = operational.step ?? (attention || heldOffline ? operational.reason : undefined);
  // Hovering anywhere on the strip shows every line in full, including the ones the compact
  // Tổng quan strip leaves out.
  const stripTitle = [
    `${machineNumberLabel(index)} · ${displayName}`,
    identity,
    operationalLabel,
    operational.step,
    operational.reason,
  ].filter(Boolean).join("\n");
  const accessibleName = [
    machineNumberLabel(index),
    displayName,
    operationalLabel,
    operational.step,
    identity,
    attention || heldOffline ? operational.reason : undefined,
    selected ? "đã chọn" : undefined,
  ].filter(Boolean).join(", ");
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
  // Frame age, not status-check age and not the connection (frameFreshness.ts).
  const freshness = useFrameFreshness(device.udid, hasView && !focused);
  const frameAge = freshness.state === "aging" || freshness.state === "stale" ? freshness.ageText : undefined;
  const frameAgeShort = freshness.state === "aging" || freshness.state === "stale" ? freshness.shortText : undefined;
  const frameLabel = [accessibleName, frameAge].filter(Boolean).join(", ");

  return (
    <article
      className={`dev-phone ${selected ? "selected" : ""} ${focused ? "focused" : ""} ${attention ? "attention" : ""}`}
      data-testid="device-tile"
      data-udid={device.udid}
      role="row"
      aria-selected={selected}
      tabIndex={0}
      aria-roledescription="thẻ thiết bị"
      aria-label={frameLabel}
      style={{ width }}
      onClick={(e) => onSelect(device.udid, e.metaKey || e.ctrlKey || e.shiftKey)}
      onKeyDown={(event) => {
        if (event.target !== event.currentTarget) return;
        if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) {
          if (!onContextMenu) return;
          event.preventDefault();
          const rect = event.currentTarget.getBoundingClientRect();
          onContextMenu(device.udid, rect.left + 16, rect.top + 16);
          return;
        }
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
      {/* The caption lives on its own light strip above the picture, never over it: white
          text over a stream disappeared on every bright screen, and anything drawn on the
          phone image hides the one thing the operator is watching. Every line truncates; the
          full text stays in the row's accessible name and each line's tooltip. */}
      <div className="dev-phone-strip" role="gridcell" title={stripTitle}>
        <div className="dev-phone-strip-head">
          <span className="dev-phone-index">{machineNumberLabel(index)}</span>
          <span className="dev-phone-name" title={displayName}>{displayName}</span>
          {frameAge && <span className="dev-phone-age" data-state={freshness.state} title={`${frameAge}. Tuổi hình: lúc màn hình gửi khung hình mới gần nhất. Không phải lần kiểm trạng thái, không phải kết nối.`}><span className="dev-phone-age-long">{frameAge}</span><span className="dev-phone-age-short" aria-hidden="true">{frameAgeShort}</span></span>}
          <span className="dev-phone-check" aria-hidden="true">{selected && <Check size={10} strokeWidth={3.5} />}</span>
        </div>
        {identity && (
          <span className="dev-phone-ident" title={identity}>
            {username && <span className="dev-phone-handle">@{username}</span>}
            {username && shownModel && " · "}
            {shownModel && <span className="dev-phone-model">{shownModel}</span>}
          </span>
        )}
        <div className="dev-phone-run">
          <DeviceStatusLine view={operational} className="dev-phone-status" />
          {operational.helperRecovery && onHelperMaintenance ? (
            <button type="button" className="link dev-phone-detail" title={operational.reason}
              onDoubleClick={event => event.stopPropagation()}
              onClick={event => { event.stopPropagation(); onHelperMaintenance(device.udid); }}>
              Khôi phục helper
            </button>
          ) : detail && <span className="dev-phone-detail" title={detail}>{detail}</span>}
        </div>
      </div>

      {/* Every tile keeps the same phone-shaped frame regardless of stream
          state or the stream's own aspect, so the grid never reflows when a
          frame arrives — the "fixed frame" the operator asked for. The canvas is
          always the first child here and never behind a condition, so a status change
          re-renders the strip without remounting the decoder surface. */}
      <div className="dev-phone-screen" role="gridcell">
        <PhoneCanvas udid={device.udid} surfaceId="tile" />
        {focused && <div className="dev-phone-focused-message" role="status">Đang mở phóng to</div>}
        {freshness.state === "stale" && <div className="dev-phone-stale" aria-hidden="true">Hình cũ</div>}
        {!focused && <StreamPlaceholder
          view={view}
          deviceName={displayName}
          onRetry={() => onPrepare(device.udid)}
        />}

        <button
          type="button"
          className="dev-phone-open"
          aria-label={`Mở màn hình ${machineNumberLabel(index)}`}
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
      </div>

      {/* Selection stays on the tile itself. The small open action is intentionally separate:
          double-click remains efficient for pointer users, while keyboard and touch users get
          one unambiguous, named control for opening the focused stream. */}
    </article>
  );
}

export const DeviceTile = memo(DeviceTileInner);
