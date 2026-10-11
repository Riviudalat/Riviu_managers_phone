import { LayoutGrid, List, Monitor, SlidersHorizontal } from "lucide-react";
import type { ViewMode } from "../viewMode";

export type { ViewMode };

const MODES: readonly { mode: ViewMode; label: string; icon: typeof List }[] = [
  { mode: "overview", label: "Tổng quan", icon: LayoutGrid },
  { mode: "window", label: "Màn hình", icon: Monitor },
  { mode: "list", label: "Danh sách", icon: List },
];

interface Props {
  viewMode: ViewMode;
  onViewMode: (v: ViewMode) => void;
  /** The horizontal "Hiển thị" panel; omitted by callers that have none. */
  displayOpen?: boolean;
  onDisplay?: () => void;
}

/**
 * View controls above the grid: the three named views and the "Hiển thị" panel toggle.
 *
 * The tile-size slider used to live here, added because the wheel gesture needs Ctrl held and
 * so nothing on screen said the size could change at all. It lives in the "Hiển thị" panel now
 * with the other display settings; Ctrl + wheel over the grid is unchanged.
 */
export function FilterToolbar({ viewMode, onViewMode, displayOpen, onDisplay }: Props) {
  return (
    <div className="filter-toolbar">
      <div className="grow" />
      <div className="view-seg" role="group" aria-label="Chế độ hiển thị">
        {MODES.map(({ mode, label, icon: Icon }) => (
          <button
            type="button"
            key={mode}
            className={viewMode === mode ? "active" : ""}
            title={label}
            aria-pressed={viewMode === mode}
            onClick={() => onViewMode(mode)}
          >
            <Icon size={15} aria-hidden="true" />
            <span>{label}</span>
          </button>
        ))}
      </div>
      {onDisplay && (
        <button
          type="button"
          className="display-toggle"
          aria-label="Hiện bảng Hiển thị"
          aria-expanded={Boolean(displayOpen)}
          aria-controls="control-display-panel"
          onClick={onDisplay}
        >
          <SlidersHorizontal size={15} aria-hidden="true" />
          <span>Hiển thị</span>
        </button>
      )}
    </div>
  );
}
