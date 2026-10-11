import type { PageId } from "../types";
import { useEffect, useState } from "react";
import { ChevronDown, PanelLeftClose, PanelLeftOpen } from "lucide-react";
import { MENU_ICONS } from "./menuIcons";
import { MENU, PINNED, menuItemOwns, type MenuItem } from "./navigationMenu";
import { useMediaQuery } from "../useMediaQuery";

const SHORT_WINDOW = "(max-height: 899px)";
const GROUPS_KEY = "riviu.sidebar.groupsOpen";
const PINNED_ITEMS = PINNED.flatMap((id) => MENU.flatMap((group) => group.items).filter((item) => item.id === id));
/** Each page has one place: a pinned page is not repeated inside its group. */
const GROUPS = MENU
  .map((group) => ({ ...group, items: group.items.filter((item) => !PINNED.includes(item.id)) }))
  .filter((group) => group.items.length > 0);

interface Props {
  page: PageId;
  selectedCount: number;
  groupMode: boolean;
  forceCompact?: boolean;
  onPage: (page: PageId) => void;
}

export function Sidebar({
  page,
  selectedCount,
  groupMode,
  forceCompact = false,
  onPage,
}: Props) {
  const [railCollapsed, setRailCollapsed] = useState(() => {
    try { return localStorage.getItem("riviu.sidebar.rail") === "true"; }
    catch { return false; }
  });
  // Below this height the six pinned shortcuts, the groups and the footer do not all fit, so a
  // group the operator has not opened or closed starts folded; on a tall screen it starts open.
  const shortWindow = useMediaQuery(SHORT_WINDOW);
  const [openGroups, setOpenGroups] = useState<Record<string, boolean>>(() => {
    try {
      const stored: unknown = JSON.parse(localStorage.getItem(GROUPS_KEY) ?? "{}");
      if (!stored || typeof stored !== "object" || Array.isArray(stored)) return {};
      return Object.fromEntries(Object.entries(stored).filter(([, value]) => typeof value === "boolean"));
    } catch {
      return {};
    }
  });
  const groupOpen = (label: string) => openGroups[label] ?? !shortWindow;
  const toggleGroup = (label: string) => setOpenGroups(current => {
    const next = { ...current, [label]: !(current[label] ?? !shortWindow) };
    try { localStorage.setItem(GROUPS_KEY, JSON.stringify(next)); } catch { /* optional preference */ }
    return next;
  });
  // A page reached from outside the sidebar (Ctrl+K, Trợ giúp, a run's source link) opens its
  // folded group, so the current page shows; a page change never folds a group.
  const currentGroup = GROUPS.find((group) => group.items.some((item) => menuItemOwns(item, page)))?.label;
  const currentGroupFolded = currentGroup !== undefined && !groupOpen(currentGroup);
  useEffect(() => {
    if (currentGroup && currentGroupFolded) setOpenGroups((current) => ({ ...current, [currentGroup]: true }));
    // Only a page change reveals; folding the current page's group by hand stays folded.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [page]);
  const toggleRail = () => setRailCollapsed(current => {
    const next = !current;
    try { localStorage.setItem("riviu.sidebar.rail", String(next)); } catch { /* optional preference */ }
    return next;
  });
  const compactRail = railCollapsed || forceCompact;
  // The accessible name is always the page's full name; the visible text is short in the pinned
  // row and the narrow rail, never an icon alone.
  const renderItem = (item: MenuItem, pinned: boolean) => {
    const Icon = MENU_ICONS[item.id];
    const current = menuItemOwns(item, page);
    return (
      <button
        key={item.id}
        type="button"
        className={`menu-item ${current ? "active" : ""}`}
        data-testid="nav-item"
        title={item.label}
        aria-label={item.label}
        aria-current={current ? "page" : undefined}
        // An entry with tabs keeps the tab already open instead of jumping back to its first one.
        onClick={() => onPage(current ? page : item.id)}
      >
        <span className="mi">{Icon && <Icon size={18} />}</span>
        <span className="menu-label">{pinned || compactRail ? item.short ?? item.label : item.label}</span>
      </button>
    );
  };
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

      <nav id="primary-navigation" className="aside-nav" aria-label="Điều hướng chính">
        <div className="menu-pinned" role="group" aria-label="Lối tắt">
          {PINNED_ITEMS.map((item) => renderItem(item, true))}
        </div>
        <div className="aside-scroll">
          {GROUPS.map((group) => (
            <section className="menu-group" key={group.label} aria-label={group.label}
              data-active={group.items.some((item) => menuItemOwns(item, page))}>
              <h2><button type="button" className="sidebar-group-toggle" aria-expanded={groupOpen(group.label)} onClick={()=>toggleGroup(group.label)}>{group.label}<ChevronDown size={14}/></button></h2>
              <div className="sidebar-group-content" data-open={compactRail || groupOpen(group.label)}
                inert={!compactRail && !groupOpen(group.label)}
                aria-hidden={!compactRail && !groupOpen(group.label)}><div>{group.items.map((item) => renderItem(item, false))}</div></div>
            </section>
          ))}
        </div>

      </nav>

      {/* Fleet counts live once, in the header's counters; the footer keeps only this window's
          own state (grid selection, Sync). */}
      <footer className="aside-stats" aria-label="Trạng thái hệ thống">
        {compactRail && <strong className="aside-compact-status" title={`Đồng bộ: ${groupMode ? "Bật" : "Tắt"}`}>Đồng bộ {groupMode ? "bật" : "tắt"}</strong>}
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
