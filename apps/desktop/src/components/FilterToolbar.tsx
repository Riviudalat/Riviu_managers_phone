import { IconGrid } from "./Icons";

export type ViewMode = "list" | "window";

interface Props {
  viewMode: ViewMode;
  onViewMode: (v: ViewMode) => void;
}

export function FilterToolbar({ viewMode, onViewMode }: Props) {
  return (
    <div className="filter-toolbar">
      <div className="grow" />
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
