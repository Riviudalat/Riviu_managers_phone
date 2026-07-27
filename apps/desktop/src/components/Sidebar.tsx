import type { PageId } from "../types";
import { MENU_ICONS, IconGrid } from "./Icons";

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

/** Menu — groups/proxy/team removed */
const MENU: {
  id: string;
  label: string;
  children: { id: PageId; label: string }[];
}[] = [
  {
    id: "common",
    label: "Thường dùng",
    children: [
      { id: "control", label: "Quản lý cửa sổ" },
      { id: "material", label: "Material" },
    ],
  },
  {
    id: "discover",
    label: "Khám phá",
    children: [
      { id: "apps", label: "App center" },
      { id: "scripts", label: "Automation" },
      { id: "sync", label: "Đồng bộ cửa sổ" },
      { id: "jobs", label: "Jobs" },
    ],
  },
  {
    id: "publish",
    label: "Xuất bản",
    children: [
      { id: "publish", label: "Publish" },
      { id: "data", label: "Data center" },
    ],
  },
  {
    id: "system",
    label: "Hệ thống",
    children: [
      { id: "account", label: "Account" },
      { id: "api", label: "API" },
      { id: "settings", label: "Settings" },
    ],
  },
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
        {!collapsed && <strong>Riviumanagersphone</strong>}
      </div>

      <div className="aside-scroll">
        {MENU.map((group) => (
          <div key={group.id} className="menu-group">
            <div className="menu-group-title">
              <span className="mi">
                <IconGrid size={18} />
              </span>
              <span>{group.label}</span>
            </div>
            {group.children.map((item) => {
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
          </div>
        ))}

        {!collapsed && (
          <div className="aside-stats">
            <h4>Dashboard</h4>
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
