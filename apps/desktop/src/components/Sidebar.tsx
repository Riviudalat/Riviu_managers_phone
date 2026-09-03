import type { PageId } from "../types";
import { PanelLeftClose, PanelLeftOpen } from "lucide-react";
import { MENU_ICONS } from "./menuIcons";

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

const MENU: { label: string; items: { id: PageId; label: string }[] }[] = [
  {
    label: "Thiết bị",
    items: [
      { id: "control", label: "Thiết bị" },
      { id: "diagnostics", label: "Chẩn đoán" },
    ],
  },
  {
    label: "Tự động hóa",
    items: [
      { id: "nurture", label: "Nuôi TikTok" },
      { id: "interaction", label: "Tương tác" },
      { id: "publish", label: "Đăng bài" },
      { id: "scripts", label: "Flow" },
      { id: "jobs", label: "Tác vụ" },
    ],
  },
  {
    label: "Tài nguyên",
    items: [
      { id: "material", label: "Kho nội dung" },
      { id: "apps", label: "Trung tâm ứng dụng" },
      { id: "data", label: "Dữ liệu" },
    ],
  },
  {
    label: "Hệ thống",
    items: [
      { id: "api", label: "API" },
      { id: "settings", label: "Cài đặt" },
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
        {!collapsed && <strong>Riviu Manager</strong>}
      </div>

      <nav className="aside-scroll" aria-label="Điều hướng chính">
        {MENU.map((group) => (
          <section className="menu-group" key={group.label} aria-label={group.label}>
            <h2>{group.label}</h2>
            {group.items.map((item) => {
              const Icon = MENU_ICONS[item.id];
              return (
                <button
                  key={item.id}
                  type="button"
                  className={`menu-item ${page === item.id ? "active" : ""}`}
                  data-testid="nav-item"
                  title={item.label}
                  aria-label={collapsed ? item.label : undefined}
                  onClick={() => onPage(item.id)}
                >
                  <span className="mi">{Icon ? <Icon size={16} /> : "›"}</span>
                  <span>{item.label}</span>
                </button>
              );
            })}
          </section>
        ))}

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
              <span>Đồng bộ</span>
              <span />
              <strong>{groupMode ? "Bật" : "Tắt"}</strong>
            </div>
          </div>
        )}
      </nav>

      <button
        type="button"
        className="aside-collapse"
        onClick={onToggleCollapse}
        title={collapsed ? "Mở rộng" : "Thu gọn"}
        aria-label={collapsed ? "Mở rộng thanh điều hướng" : "Thu gọn thanh điều hướng"}
      >
        {collapsed ? <PanelLeftOpen size={17} /> : <PanelLeftClose size={17} />}
      </button>
    </aside>
  );
}
