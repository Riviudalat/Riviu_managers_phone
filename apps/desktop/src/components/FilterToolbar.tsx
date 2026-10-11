import { List, Monitor } from "lucide-react";
import type { ViewMode } from "../viewMode";

export type { ViewMode };

const MODES: readonly { mode: ViewMode; label: string; icon: typeof List }[] = [
  { mode: "window", label: "Màn hình", icon: Monitor },
  { mode: "list", label: "Danh sách", icon: List },
];

interface Props {
  viewMode: ViewMode;
  onViewMode: (v: ViewMode) => void;
}

/**
 * View controls above the grid: the wall and the table.
 *
 * Display settings (tile size, stream quality, connection and group filters) live in the left
 * "Cài đặt" panel beside the wall (GF1), as on GenFarmer; Ctrl + wheel over the grid is
 * unchanged.
 */
export function FilterToolbar({ viewMode, onViewMode }: Props) {
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
    </div>
  );
}
