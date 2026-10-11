import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { FolderPlus, Play, RefreshCcw, SlidersHorizontal, X } from "lucide-react";

import type { ActiveGroupSync, GroupSyncReadiness } from "../groupSync";
import type { DeviceGroup, DeviceInfo, DeviceMeta } from "../types";
import { tileName } from "../deviceNaming";
import { toastError } from "../toastStore";
import { IconPhone } from "./Icons";
import { GroupSyncSection } from "./settings/GroupSyncSection";
import "../styles/selection-bar.css";

interface Props {
  selected: DeviceInfo[];
  /** Selected phones the active filter keeps off screen; commands still reach them. */
  hiddenCount: number;
  groups: DeviceGroup[];
  activeSync: ActiveGroupSync | null;
  readiness: GroupSyncReadiness | null;
  resolvedMasterUdid: string | null;
  onMasterChange: (udid: string | null) => void;
  onEnableSync: (masterUdid: string) => void;
  onDisableSync: () => void;
  deviceNumbers?: Map<string, number>;
  metas?: Map<string, DeviceMeta>;
  onOpen: () => void | Promise<void>;
  onRunFlow: () => void;
  onGroupTools: () => void;
  groupToolsOpen: boolean;
  onAddToGroup: (groupId: string) => void | Promise<void>;
  onManageGroups: () => void;
  onClear: () => void;
}

/** Room the toast needs to stay above the bar; read by `.activity-center-current`. */
const CLEARANCE = "--selection-bar-clearance";

/**
 * The one home for commands on the selected phones, sticky under the wall.
 *
 * It says how many selected phones the filter hides, because every command here reaches
 * them too: a bar that only counted what is on screen would send work to phones the
 * operator cannot see. Clearing the selection stops Sync, so Sync lives here as well.
 */
export function SelectionBar({
  selected,
  hiddenCount,
  groups,
  activeSync,
  readiness,
  resolvedMasterUdid,
  onMasterChange,
  onEnableSync,
  onDisableSync,
  deviceNumbers,
  metas,
  onOpen,
  onRunFlow,
  onGroupTools,
  groupToolsOpen,
  onAddToGroup,
  onManageGroups,
  onClear,
}: Props) {
  const barRef = useRef<HTMLDivElement>(null);
  const [syncOpen, setSyncOpen] = useState(false);
  const syncTrigger = useRef<HTMLButtonElement>(null);
  const [groupMenuOpen, setGroupMenuOpen] = useState(false);
  const groupTrigger = useRef<HTMLButtonElement>(null);
  const groupMenuRef = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const bar = barRef.current;
    if (!bar) return;
    const root = document.documentElement;
    let frame = 0;
    const measure = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const top = bar.getBoundingClientRect().top;
        root.style.setProperty(CLEARANCE, `${Math.max(0, Math.round(window.innerHeight - top))}px`);
      });
    };
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(bar);
    window.addEventListener("resize", measure);
    document.addEventListener("scroll", measure, true);
    return () => {
      cancelAnimationFrame(frame);
      observer?.disconnect();
      window.removeEventListener("resize", measure);
      document.removeEventListener("scroll", measure, true);
      root.style.removeProperty(CLEARANCE);
    };
  }, []);

  useEffect(() => {
    if (!groupMenuOpen) return;
    const away = (event: PointerEvent) => {
      if (event.target instanceof Node && !groupMenuRef.current?.contains(event.target)) setGroupMenuOpen(false);
    };
    document.addEventListener("pointerdown", away, true);
    return () => document.removeEventListener("pointerdown", away, true);
  }, [groupMenuOpen]);

  const ready = new Set(readiness?.readyUdids ?? []);
  const failures = readiness?.failures ?? {};
  const labelFor = (device: DeviceInfo) => {
    const meta = metas?.get(device.udid);
    const number = deviceNumbers?.get(device.udid) ?? meta?.number;
    return `${number ? `Máy ${number} · ` : ""}${tileName(device, meta)}${meta?.handle ? ` · @${meta.handle.replace(/^@+/, "")}` : ""}`;
  };
  const masterValid = Boolean(
    resolvedMasterUdid && selected.some((device) => device.udid === resolvedMasterUdid),
  );
  const disconnected = selected.filter((device) => device.status === "disconnected");
  const enableReason = selected.length < 2
    ? "Chọn ít nhất hai máy để bật đồng bộ."
    : !masterValid
      ? "Chọn lại một máy chính trong phạm vi."
      : disconnected.length > 0
        ? `${disconnected.map(labelFor).join(", ")} đang mất kết nối.`
        : null;
  const status = !activeSync
    ? "Đang tắt"
    : readiness?.state === "active"
      ? `Đang hoạt động ${readiness.readyUdids.length}/${activeSync.targetUdids.length}`
      : readiness?.state === "degraded"
        ? `Cần xử lý ${Object.keys(failures).length} máy`
        : `Đang chuẩn bị ${readiness?.readyUdids.length ?? 0}/${activeSync.targetUdids.length}`;

  const closeSyncPanel = () => {
    setSyncOpen(false);
    syncTrigger.current?.focus();
  };
  const closeGroupMenu = () => {
    setGroupMenuOpen(false);
    groupTrigger.current?.focus();
  };

  return (
    <div ref={barRef} className="selection-bar profile-toolbar" role="group" aria-label="Thao tác với máy đã chọn">
      <span className="selection-bar-count">
        <strong>{selected.length} máy đã chọn</strong>
        {hiddenCount > 0 && <span className="selection-bar-hidden">{hiddenCount} máy đã chọn đang bị bộ lọc ẩn</span>}
      </span>
      <button
        type="button"
        className="tb-btn primary"
        onClick={async () => {
          try {
            await onOpen();
          } catch (error) {
            toastError("Khởi động thất bại", error);
          }
        }}
        title={`Mở luồng xem cho ${selected.length} máy đã chọn`}
      >
        <IconPhone size={16} />
        Mở màn hình
      </button>
      <button
        type="button"
        className={`tb-btn ${activeSync ? "active" : ""}`}
        ref={syncTrigger}
        aria-expanded={syncOpen}
        aria-controls="toolbar-sync-panel"
        onClick={() => { setGroupMenuOpen(false); setSyncOpen((open) => !open); }}
        title="Đồng bộ thao tác trên nhóm máy đã chọn"
      >
        <RefreshCcw size={16} aria-hidden="true" />
        {activeSync ? `Đồng bộ · ${activeSync.targetUdids.length} máy` : "Đồng bộ thao tác"}
      </button>
      <button type="button" className="tb-btn" onClick={onRunFlow} title="Mở Flow thiết bị với các máy đã chọn; chạy sau khi xác nhận">
        <Play size={16} aria-hidden="true" />Chạy Flow…
      </button>
      <button type="button" className={`tb-btn ${groupToolsOpen ? "active" : ""}`} aria-expanded={groupToolsOpen} data-group-tools onClick={onGroupTools} title="Công cụ nhóm: phân phối văn bản/tệp…">
        <SlidersHorizontal size={16} aria-hidden="true" />Công cụ nhóm…
      </button>
      <div ref={groupMenuRef} className="selection-bar-groups">
        <button
          type="button"
          ref={groupTrigger}
          className={`tb-btn ${groupMenuOpen ? "active" : ""}`}
          aria-expanded={groupMenuOpen}
          aria-controls="selection-bar-group-list"
          onClick={() => { setSyncOpen(false); setGroupMenuOpen((open) => !open); }}
        >
          <FolderPlus size={16} aria-hidden="true" />Thêm vào nhóm
        </button>
        {groupMenuOpen && (
          <div
            id="selection-bar-group-list"
            className="selection-bar-group-list"
            role="group"
            aria-label="Chọn nhóm"
            onKeyDown={(event) => {
              if (event.key !== "Escape") return;
              event.preventDefault();
              event.stopPropagation();
              closeGroupMenu();
            }}
          >
            {groups.length === 0 && <p>Chưa có nhóm nào.</p>}
            {groups.map((group) => (
              <button
                key={group.id}
                type="button"
                onClick={() => {
                  closeGroupMenu();
                  void onAddToGroup(group.id);
                }}
              >
                <span className="device-menu-dot" style={{ background: group.color }} />
                {group.name}
              </button>
            ))}
            <button type="button" className="selection-bar-manage" onClick={() => { setGroupMenuOpen(false); onManageGroups(); }}>
              Quản lý nhóm…
            </button>
          </div>
        )}
      </div>
      {syncOpen && (
        <section
          id="toolbar-sync-panel"
          className="toolbar-sync-panel"
          aria-label="Điều khiển đồng bộ"
          onKeyDown={(event) => {
            if (event.key !== "Escape") return;
            event.preventDefault();
            event.stopPropagation();
            closeSyncPanel();
          }}
        >
          <div className="toolbar-sync-heading">
            <div><strong>Đồng bộ thao tác</strong><span>{status}</span></div>
            <button type="button" className="tb-btn" onClick={closeSyncPanel}>Đóng bảng</button>
          </div>
          <p className="hint">Máy chính là màn hình điều khiển. Thao tác chỉ mở sau khi toàn bộ phiên sẵn sàng.</p>
          <label className="toolbar-sync-master">
            Máy chính
            <select value={resolvedMasterUdid ?? ""} onChange={(event) => onMasterChange(event.target.value || null)}>
              {!resolvedMasterUdid && <option value="">Chọn máy chính</option>}
              {selected.map((device) => <option key={device.udid} value={device.udid}>{labelFor(device)}</option>)}
            </select>
          </label>
          <div className="toolbar-sync-targets" aria-label="Máy nhận thao tác">
            {selected.map((device) => {
              const master = device.udid === (activeSync?.masterUdid ?? resolvedMasterUdid);
              const failure = failures[device.udid];
              const sessionState = device.status === "disconnected"
                ? "Mất kết nối"
                : failure
                  ? "Cần xử lý"
                  : activeSync
                    ? ready.has(device.udid) ? "Sẵn sàng" : "Đang chuẩn bị"
                    : "Đã chọn";
              return (
                <div className="toolbar-sync-target" key={device.udid} data-state={failure ? "error" : sessionState.toLowerCase()}>
                  <div><strong>{labelFor(device)}</strong><span>{master ? "Máy chính" : "Máy nhận"}</span></div>
                  <div className="toolbar-sync-target-state"><span>{sessionState}</span>{failure && <small>{failure}</small>}</div>
                </div>
              );
            })}
          </div>
          <div className="toolbar-sync-actions">
            {activeSync ? (
              <button type="button" className="tb-btn danger" onClick={onDisableSync}>Tắt đồng bộ</button>
            ) : (
              <button
                type="button"
                className="tb-btn primary"
                disabled={Boolean(enableReason)}
                onClick={() => {
                  if (!resolvedMasterUdid) return;
                  onEnableSync(resolvedMasterUdid);
                  closeSyncPanel();
                }}
              >
                Bật đồng bộ thao tác
              </button>
            )}
            {enableReason && <p role="status">{enableReason}</p>}
          </div>
          <details className="toolbar-sync-options"><summary>Độ trễ và độ lệch thao tác</summary><GroupSyncSection /></details>
        </section>
      )}
      <div className="grow" />
      <button type="button" className="tb-btn" onClick={onClear} title="Bỏ chọn mọi máy, kể cả máy bộ lọc đang ẩn">
        <X size={16} aria-hidden="true" />Bỏ chọn
      </button>
    </div>
  );
}
