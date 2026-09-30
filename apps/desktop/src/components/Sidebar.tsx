import type { PageId } from "../types";
import { useState } from "react";
import { ChevronDown, PanelLeftClose, PanelLeftOpen } from "lucide-react";
import { MENU_ICONS } from "./menuIcons";

interface Props {
  page: PageId;
  selectedCount: number;
  total: number;
  readyCount: number;
  groupMode: boolean;
  forceCompact?: boolean;
  onPage: (page: PageId) => void;
}

const MENU: { label: string; items: { id: PageId; label: string }[] }[] = [
  {
    label: "Thiết bị",
    items: [
      { id: "control", label: "Control Center" },
    ],
  },
  {
    label: "Automation",
    items: [
      { id: "nurture", label: "Nuôi TikTok" },
      { id: "interaction", label: "Tương tác" },
      { id: "publish", label: "Đăng bài" },
      { id: "myApps", label: "My Apps" },
      { id: "jobs", label: "Lượt chạy" },
      { id: "savedTasks", label: "Tác vụ đã lưu" },
      { id: "scripts", label: "Flow thiết bị" },
    ],
  },
  {
    label: "Tài nguyên",
    items: [
      { id: "accounts", label: "Quản lý tài khoản" },
      { id: "schedules", label: "Lịch chạy" },
      { id: "material", label: "Kho nội dung" },
      { id: "apps", label: "Trung tâm ứng dụng" },
    ],
  },
  {
    label: "Hệ thống",
    items: [
      { id: "api", label: "API" },
      { id: "diagnostics", label: "Chẩn đoán" },
      { id: "settings", label: "Cài đặt" },
      { id: "help", label: "Trợ giúp" },
    ],
  },
];

export function Sidebar({
  page,
  selectedCount,
  total,
  readyCount,
  groupMode,
  forceCompact = false,
  onPage,
}: Props) {
  const [railCollapsed, setRailCollapsed] = useState(() => {
    try { return localStorage.getItem("riviu.sidebar.rail") === "true"; }
    catch { return false; }
  });
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>(() => {
    try {
      const stored: unknown = JSON.parse(localStorage.getItem("riviu.sidebar.groups") ?? "{}");
      if (!stored || typeof stored !== "object" || Array.isArray(stored)) return {};
      return Object.fromEntries(Object.entries(stored).filter(([, value]) => typeof value === "boolean"));
    } catch {
      return {};
    }
  });
  const toggleGroup = (label: string) => setCollapsed(current => {
    const next = { ...current, [label]: !current[label] };
    try { localStorage.setItem("riviu.sidebar.groups", JSON.stringify(next)); } catch { /* optional preference */ }
    return next;
  });
  const toggleRail = () => setRailCollapsed(current => {
    const next = !current;
    try { localStorage.setItem("riviu.sidebar.rail", String(next)); } catch { /* optional preference */ }
    return next;
  });
  const compactRail = railCollapsed || forceCompact;
  return (
    <aside className="aside" aria-label="Riviu Manager" data-rail-collapsed={compactRail}>
      <div className="aside-logo">
        <img src="/logo.jpg" alt="" />
        <strong>Riviu Manager<small>PHONE WORKSPACE</small></strong>
        <button type="button" className="aside-rail-toggle" aria-controls="primary-navigation"
          aria-expanded={!compactRail}
          aria-label={compactRail ? "Mở rộng thanh điều hướng" : "Thu gọn thanh điều hướng"}
          title={compactRail ? "Mở rộng thanh điều hướng" : "Thu gọn thanh điều hướng"}
          onClick={toggleRail}>
          {compactRail ? <PanelLeftOpen size={18} aria-hidden="true" /> : <PanelLeftClose size={18} aria-hidden="true" />}
        </button>
      </div>

      <nav id="primary-navigation" className="aside-scroll" aria-label="Điều hướng chính">
        {MENU.map((group) => (
          <section className="menu-group" key={group.label} aria-label={group.label}
            data-active={group.items.some((item) => item.id === page)}>
            <h2><button type="button" className="sidebar-group-toggle" aria-expanded={!collapsed[group.label]} onClick={()=>toggleGroup(group.label)}>{group.label}<ChevronDown size={14}/></button></h2>
            <div className="sidebar-group-content" data-open={compactRail || !collapsed[group.label]}
              inert={!compactRail && !!collapsed[group.label]}
              aria-hidden={!compactRail && !!collapsed[group.label]}><div>{group.items.map((item) => {
              const Icon = MENU_ICONS[item.id];
              return (
                <button
                  key={item.id}
                  type="button"
                  className={`menu-item ${page === item.id ? "active" : ""}`}
                  data-testid="nav-item"
                  title={item.label}
                  aria-label={item.label}
                  aria-current={page === item.id ? "page" : undefined}
                  onClick={() => onPage(item.id)}
                >
                  <span className="mi">{Icon && <Icon size={18} />}</span>
                  <span className="menu-label">{item.label}</span>
                </button>
              );
            })}</div></div>
          </section>
        ))}

      </nav>

      <footer className="aside-stats" aria-label="Trạng thái hệ thống">
        {compactRail && <strong className="aside-compact-status" title={`${readyCount}/${total} máy sẵn sàng`}>{readyCount}/{total}</strong>}
        <h4>Kết nối</h4>
        <div className="aside-stat-row">
          <span>Sẵn sàng</span>
          <span />
          <strong>{readyCount}/{total}</strong>
        </div>
        {page === "control" && <div className="aside-stat-row">
          <span>Đã chọn trong lưới</span>
          <span />
          <strong>{selectedCount}</strong>
        </div>}
        <div className="aside-stat-row">
          <span>Đồng bộ</span>
          <span />
          <strong>{groupMode ? "Bật" : "Tắt"}</strong>
        </div>
      </footer>
    </aside>
  );
}
