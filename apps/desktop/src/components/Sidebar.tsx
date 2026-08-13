import type { PageId } from "../types";
import { MENU_ICONS } from "./Icons";

interface Props {
  page: PageId;
  collapsed: boolean;
  selectedCount: number;
  total: number;
  readyCount: number;
  groupMode: boolean;
  onPage: (page: PageId) => void;
  onToggleCollapse: () => void;
}

/** Menu — one flat list; groups/proxy/team removed */
const MENU: { id: PageId; label: string }[] = [
  { id: "control", label: "Quản lý cửa sổ" },
  { id: "material", label: "Kho nội dung" },
  { id: "apps", label: "Trung tâm ứng dụng" },
  { id: "scripts", label: "Flow" },
  { id: "sync", label: "Đồng bộ cửa sổ" },
  { id: "jobs", label: "Tác vụ" },
  { id: "publish", label: "Đăng bài" },
  { id: "data", label: "Dữ liệu" },
  { id: "account", label: "Tài khoản" },
  { id: "api", label: "API" },
  { id: "settings", label: "Cài đặt" },
];

export function Sidebar({
  page,
  collapsed,
  selectedCount,
  total,
  readyCount,
  groupMode,
  onPage,
  onToggleCollapse,
}: Props) {
  return (
    <aside className={`aside ${collapsed ? "collapsed" : ""}`}>
      <div className="aside-logo">
        <img src="/logo.jpg" alt="" />
        {!collapsed && <strong>Riviu Manager</strong>}
      </div>

      <div className="aside-scroll">
        {MENU.map((item) => {
          const Icon = MENU_ICONS[item.id];
          return (
            <button
              key={item.id}
              type="button"
              className={`menu-item ${page === item.id ? "active" : ""}`}
              title={item.label}
              onClick={() => onPage(item.id)}
            >
              <span className="mi">{Icon ? <Icon size={16} /> : "›"}</span>
              <span>{item.label}</span>
            </button>
          );
        })}

        {!collapsed && (
          <div className="aside-stats">
            <h4>Tổng quan</h4>
            <div className="aside-stat-row">
              <span>Thiết bị</span>
              <span />
              <strong>
                {readyCount}/{total}
              </strong>
            </div>
            <div className="aside-stat-row">
              <span>Đã chọn</span>
              <span />
              <strong>{selectedCount}</strong>
            </div>
            <div className="aside-stat-row">
              <span>Sync</span>
              <span />
              <strong>{groupMode ? "ON" : "OFF"}</strong>
            </div>
          </div>
        )}
      </div>

      <div className="aside-collapse" onClick={onToggleCollapse} title="Thu gọn">
        {collapsed ? "›" : "‹"}
      </div>
    </aside>
  );
}
