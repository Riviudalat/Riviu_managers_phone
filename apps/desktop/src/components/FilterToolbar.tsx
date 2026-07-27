import { IconGrid } from "./Icons";

export type ViewMode = "list" | "window";

interface Props {
  query: string;
  connection: string;
  status: string;
  viewMode: ViewMode;
  tileSize: string;
  onQuery: (v: string) => void;
  onConnection: (v: string) => void;
  onStatus: (v: string) => void;
  onViewMode: (v: ViewMode) => void;
  onTileSize: (v: string) => void;
}

export function FilterToolbar({
  query,
  connection,
  status,
  viewMode,
  tileSize,
  onQuery,
  onConnection,
  onStatus,
  onViewMode,
  onTileSize,
}: Props) {
  return (
    <div className="filter-toolbar">
      <select value={connection} onChange={(e) => onConnection(e.target.value)}>
        <option value="">Nhóm / Connection</option>
        <option value="usb">USB</option>
        <option value="wifi">Wi‑Fi</option>
        <option value="mock">Mock</option>
      </select>
      <select value={status} onChange={(e) => onStatus(e.target.value)}>
        <option value="">Trạng thái</option>
        <option value="ready">Running</option>
        <option value="connected">Online</option>
        <option value="preparing">Starting</option>
        <option value="error">Error</option>
      </select>
      <input
        value={query}
        onChange={(e) => onQuery(e.target.value)}
        placeholder="Tên cửa sổ / UDID…"
      />
      <select value={tileSize} onChange={(e) => onTileSize(e.target.value)}>
        <option value="thumbnail">S</option>
        <option value="medium">M</option>
        <option value="large">L</option>
        <option value="extraLarge">XL</option>
      </select>
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
