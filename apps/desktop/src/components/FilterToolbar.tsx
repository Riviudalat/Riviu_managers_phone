import { LayoutGrid, List } from "lucide-react";

export type ViewMode = "list" | "window";

interface Props {
  viewMode: ViewMode;
  onViewMode: (v: ViewMode) => void;
}

/**
 * View controls above the grid: list or windows, and nothing else.
 *
 * The tile-size slider used to live here, added because the wheel gesture needs Ctrl held and
 * so nothing on screen said the size could change at all. It is gone at the operator's request
 * — the gesture is the control they want — and `Ctrl + lăn chuột` is now stated as the grid's
 * own tooltip in `App.tsx` rather than as a slider's. The gesture itself did not change: same
 * `TILE_ZOOM` range, same clamp, same persisted key.
 */
export function FilterToolbar({ viewMode, onViewMode }: Props) {
  return (
    <div className="filter-toolbar">
      <div className="grow" />
      <div className="view-seg" role="group" aria-label="Chế độ hiển thị">
        <button
          type="button"
          className={viewMode === "list" ? "active" : ""}
          title="Danh sách"
          aria-label="Hiển thị dạng danh sách"
          aria-pressed={viewMode === "list"}
          onClick={() => onViewMode("list")}
        >
          <List size={15} aria-hidden="true" />
        </button>
        <button
          type="button"
          className={viewMode === "window" ? "active" : ""}
          title="Cửa sổ stream"
          aria-label="Hiển thị dạng lưới stream"
          aria-pressed={viewMode === "window"}
          onClick={() => onViewMode("window")}
        >
          <LayoutGrid size={15} aria-hidden="true" />
        </button>
      </div>
    </div>
  );
}
