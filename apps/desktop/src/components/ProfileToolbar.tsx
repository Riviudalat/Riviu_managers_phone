import { useEffect, useId, useRef, useState } from "react";
import { ChevronDown, FolderKanban, MoreHorizontal, SlidersHorizontal, Wrench } from "lucide-react";

import type { DeviceInfo } from "../types";
import { toastError } from "../toastStore";
import { IconPhone, IconRefresh } from "./Icons";

interface Props {
  selected: DeviceInfo[];
  deviceCount: number;
  onStart: () => void | Promise<void>;
  onInstall: () => void | Promise<void>;
  onHelperMaintenance?: () => void | Promise<void>;
  onQuarantineMaintenance?: () => void | Promise<void>;
  onRefresh: () => void | Promise<void>;
  onGroupTools: () => void;
  onGroups: () => void;
  groupsOpen: boolean;
  groupToolsOpen: boolean;
}

/**
 * Fleet-wide commands. While phones are selected, the commands that act on the selection
 * (open, Sync, group tools, clear) live only in `SelectionBar`, so none has two homes.
 */
export function ProfileToolbar({
  selected,
  deviceCount,
  onStart,
  onInstall,
  onHelperMaintenance,
  onQuarantineMaintenance,
  onRefresh,
  onGroupTools,
  onGroups,
  groupsOpen,
  groupToolsOpen,
}: Props) {
  const any = selected.length;
  const canBatch = deviceCount > 0;
  const scope = any ? `đã chọn (${any})` : `toàn bộ (${deviceCount})`;
  // Below the width that fits every fleet action beside a usable search field, Nhóm, Công cụ,
  // Bảo trì and Quét lại move behind "⋯" (controls.css decides when; this state only says
  // whether that menu is open). The controls are the same elements either way.
  const [moreOpen, setMoreOpen] = useState(false);
  const moreId = useId();
  const moreRef = useRef<HTMLDivElement>(null);
  const moreToggle = useRef<HTMLButtonElement>(null);
  // Set when "⋯" lost focus because it stopped being rendered (the browser blurs a focused
  // element that leaves the layout), so the tier change below can hand focus on.
  const toggleHiddenWhileFocused = useRef(false);
  // The same for an inline action that goes behind "⋯".
  const actionHiddenWhileFocused = useRef(false);
  useEffect(() => {
    if (!moreOpen) return;
    const onPointerDown = (event: PointerEvent) => {
      if (!moreRef.current?.contains(event.target as Node)) setMoreOpen(false);
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [moreOpen]);
  // The tier is CSS (a media query and the rail pin), so watch its one visible effect: "⋯" being
  // rendered or not. Crossing the boundary closes the menu, and focus never stays on something
  // the new tier hides — "⋯" itself when the actions come back inline, or an action that just
  // went behind "⋯".
  useEffect(() => {
    const toggle = moreToggle.current;
    if (!toggle || typeof ResizeObserver === "undefined") return;
    let overflowTier = toggle.getClientRects().length > 0;
    const observer = new ResizeObserver(() => {
      const now = toggle.getClientRects().length > 0;
      if (now === overflowTier) return;
      overflowTier = now;
      setMoreOpen(false);
      const active = document.activeElement;
      const secondary = document.getElementById(moreId);
      const lostWithToggle = active === toggle || toggleHiddenWhileFocused.current;
      const lostWithAction = (active instanceof HTMLElement && !!secondary?.contains(active)) || actionHiddenWhileFocused.current;
      toggleHiddenWhileFocused.current = false;
      actionHiddenWhileFocused.current = false;
      if (!now && lostWithToggle) {
        secondary?.querySelector<HTMLElement>("button:not(:disabled), summary")?.focus();
      } else if (now && lostWithAction) {
        toggle.focus();
      }
    });
    observer.observe(toggle);
    return () => observer.disconnect();
  }, [moreId]);
  /** Runs an action from the menu: closes it and parks focus on "⋯" first, so a dialog the action
   *  opens returns focus to a control that is still on screen. A no-op when the menu is inline. */
  const fromMore = (action: () => void) => {
    if (moreOpen) {
      setMoreOpen(false);
      moreToggle.current?.focus();
    }
    action();
  };

  return (
    <div className="profile-toolbar" role="group" aria-label="Thao tác thiết bị">
      {!any && <button
        type="button"
        className="tb-btn primary"
        disabled={!canBatch}
        onClick={async () => {
          try {
            await onStart();
          } catch (error) {
            toastError("Khởi động thất bại", error);
          }
        }}
        title={`Mở luồng xem cho ${scope}`}
      >
        <IconPhone size={16} />
        Mở {scope}
      </button>}
      <div ref={moreRef} className="toolbar-more" onKeyDown={(event) => {
        if (event.key !== "Escape" || event.defaultPrevented || !moreOpen) return;
        event.preventDefault();
        setMoreOpen(false);
        moreToggle.current?.focus();
      }}>
        <button ref={moreToggle} type="button" className="tb-btn toolbar-more-toggle" aria-label="Thêm thao tác thiết bị"
          onFocus={() => { toggleHiddenWhileFocused.current = false; }}
          onBlur={(event) => { toggleHiddenWhileFocused.current = !event.relatedTarget && event.currentTarget.getClientRects().length === 0; }}
          title="Thêm thao tác: Nhóm, Công cụ, Bảo trì, Quét lại" aria-expanded={moreOpen} aria-controls={moreId}
          onClick={() => setMoreOpen((open) => !open)}>
          <MoreHorizontal size={18} aria-hidden="true" />
        </button>
        <div id={moreId} className="toolbar-secondary" data-open={moreOpen}
          onFocus={() => { actionHiddenWhileFocused.current = false; }}
          onBlur={(event) => { actionHiddenWhileFocused.current = !event.relatedTarget && event.target.getClientRects().length === 0; }}>
          <button type="button" className={`tb-btn ${groupsOpen ? "active" : ""}`} aria-expanded={groupsOpen} onClick={() => fromMore(onGroups)} title="Chia fleet thành nhóm — mỗi máy thuộc đúng một nhóm">
            <FolderKanban size={16} aria-hidden="true" /><span className="tb-label">Nhóm</span>
          </button>
          {!any && <button type="button" className={`tb-btn ${groupToolsOpen ? "active" : ""}`} aria-expanded={groupToolsOpen} data-group-tools onClick={() => fromMore(onGroupTools)} title="Công cụ nhóm: phân phối văn bản/tệp…">
            <SlidersHorizontal size={16} aria-hidden="true" /><span className="tb-label">Công cụ</span>
          </button>}
          <details className="toolbar-maintenance" onKeyDown={(event) => {
            if (event.key !== "Escape") return;
            event.preventDefault();
            event.currentTarget.removeAttribute("open");
            event.currentTarget.querySelector("summary")?.focus();
          }}>
            <summary tabIndex={0} title="Bảo trì thiết bị"><Wrench size={15} aria-hidden="true" /><span className="tb-label">Bảo trì</span><ChevronDown size={14} aria-hidden="true" /></summary>
            <div className="toolbar-maintenance-menu">
              <strong>Bảo trì thiết bị</strong>
              <span>{any ? `${any} máy đã chọn` : "Các máy đang kết nối"}</span>
              <button type="button" className="tb-btn" disabled={!canBatch} onClick={async (event) => {
                event.currentTarget.closest("details")?.removeAttribute("open");
                if (moreOpen) { setMoreOpen(false); moreToggle.current?.focus(); }
                try { await onInstall(); } catch (error) { toastError("Sửa agent thất bại", error); }
              }} title={`Cài hoặc khôi phục Riviu Agent cho ${scope}`}>
                <Wrench size={16} aria-hidden="true" />Sửa Riviu Agent
              </button>
              <button type="button" className="tb-btn" disabled={!canBatch || !onHelperMaintenance} onClick={async (event) => {
                event.currentTarget.closest("details")?.removeAttribute("open");
                if (moreOpen) { setMoreOpen(false); moreToggle.current?.focus(); }
                try { await onHelperMaintenance?.(); } catch (error) { toastError("Khôi phục helper thất bại", error); }
              }} title={any ? "Chuẩn bị và xác nhận khôi phục helper trên các máy Android đã chọn" : "Chuẩn bị và xác nhận khôi phục helper trên các máy Android đang báo cần khôi phục helper"}>
                <Wrench size={16} aria-hidden="true" />Khôi phục helper
              </button>
              <button type="button" className="tb-btn" disabled={!canBatch || !onQuarantineMaintenance} onClick={async (event) => {
                event.currentTarget.closest("details")?.removeAttribute("open");
                if (moreOpen) { setMoreOpen(false); moreToggle.current?.focus(); }
                try { await onQuarantineMaintenance?.(); } catch (error) { toastError("Khôi phục phiên điều khiển thất bại", error); }
              }} title="Khôi phục phiên TikTok được giữ lại trên các máy Android đang kết nối trong phạm vi; kế hoạch đã gửi chỉ được tra cứu">
                <Wrench size={16} aria-hidden="true" />Khôi phục phiên điều khiển
              </button>
            </div>
          </details>
          <button type="button" className="tb-btn refresh" onClick={() => fromMore(() => void onRefresh())} title="Quét lại thiết bị" aria-label="Quét lại thiết bị">
            <IconRefresh size={15} aria-hidden="true" /><span className="tb-label toolbar-more-only">Quét lại thiết bị</span>
          </button>
        </div>
      </div>
    </div>
  );
}
