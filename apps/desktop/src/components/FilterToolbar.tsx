import { IconGrid } from "./Icons";
import { TILE_ZOOM, clampZoom } from "../zoom";

export type ViewMode = "list" | "window";

interface Props {
  viewMode: ViewMode;
  onViewMode: (v: ViewMode) => void;
  tileWidth: number;
  onTileWidth: (width: number) => void;
}

export function FilterToolbar({ viewMode, onViewMode, tileWidth, onTileWidth }: Props) {
  return (
    <div className="filter-toolbar">
      <div className="grow" />
      {/* The tile size had a wheel gesture and no visible control, and the gesture needs
          Ctrl held (`wheelWantsZoom`) — so nothing on screen said the size could change
          at all. The slider is the discoverable half; both write the same clamped value
          through the same range, so they cannot disagree. */}
      <label className="tile-zoom" title="Cỡ màn hình xem (Ctrl + lăn chuột cũng được)">
        <span>Cỡ</span>
        <input
          type="range"
          min={TILE_ZOOM.min}
          max={TILE_ZOOM.max}
          step={10}
          value={tileWidth}
          aria-label="Cỡ màn hình xem"
          onChange={(event) => onTileWidth(clampZoom(TILE_ZOOM, Number(event.target.value)))}
        />
      </label>
      <div className="view-seg" role="group" aria-label="View mode">
        <button
          type="button"
          className={viewMode === "list" ? "active" : ""}
          title="Danh sách"
          onClick={() => onViewMode("list")}
        >
          ≡
        </button>
        <button
          type="button"
          className={viewMode === "window" ? "active" : ""}
          title="Cửa sổ stream"
          onClick={() => onViewMode("window")}
        >
          <IconGrid size={15} />
        </button>
      </div>
    </div>
  );
}
