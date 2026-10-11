import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { ChevronUp, FolderPlus, Play, RefreshCcw, SlidersHorizontal, X } from "lucide-react";

import type { ActiveGroupSync, GroupSyncReadiness } from "../groupSync";
import type { DeviceGroup, DeviceInfo, DeviceMeta } from "../types";
import type { ExcludedDevice } from "../selectionEligibility";
import { machineNumberLabel, tileName } from "../deviceNaming";
import { toastError } from "../toastStore";
import { ExcludedDevicesNotice } from "./ExcludedDevicesNotice";
import { IconPhone } from "./Icons";
import { GroupSyncSection } from "./settings/GroupSyncSection";
import "../styles/selection-bar.css";

interface Props {
  selected: DeviceInfo[];
  /** Selected phones the active filter keeps off screen; commands still reach them. */
  hiddenCount: number;
  /** Which selected phones those are, for the full list. */
  hiddenUdids?: ReadonlySet<string>;
  /** Group names per phone, for the full list. */
  groupNames?: ReadonlyMap<string, string[]>;
  /** Phones that left the selection by themselves (busy, disconnected) and why. */
  excluded?: ExcludedDevice[];
  onDismissExcluded?: () => void;
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
 *
 * The count opens the full list (also Ctrl+Shift+L), and phones that left the selection
 * because they went busy or offline stay named under "Bị loại" until the operator clears
 * them. With nothing left selected the commands are off: an empty selection must never
 * become "every phone" behind the operator's back.
 */
export function SelectionBar({
  selected,
  hiddenCount,
  hiddenUdids,
  groupNames,
  excluded = [],
  onDismissExcluded,
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
  const [listOpen, setListOpen] = useState(false);
  const listTrigger = useRef<HTMLButtonElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const none = selected.length === 0;

  // Ctrl+Shift+L opens the full list from anywhere on the page, the filter's search box
  // included (the chord edits no text there), but never from a multi-line or phone-keyboard
  // field, nor while another dialog owns the screen. Escape inside the list closes it.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || event.altKey || !event.shiftKey) return;
      if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== "l") return;
      const target = event.target as HTMLElement | null;
      if (target?.closest?.("textarea, [contenteditable='true'], .automation-host")) return;
      if (document.querySelector('[aria-modal="true"]')) return;
      event.preventDefault();
      setSyncOpen(false);
      setGroupMenuOpen(false);
      setListOpen((open) => {
        if (open) listTrigger.current?.focus();
        return !open;
      });
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  useEffect(() => {
    if (!listOpen) return;
    listRef.current?.focus({ preventScroll: true });
    const away = (event: PointerEvent) => {
      if (event.target instanceof Node && !listRef.current?.contains(event.target)
        && !listTrigger.current?.contains(event.target)) setListOpen(false);
    };
    document.addEventListener("pointerdown", away, true);
    return () => document.removeEventListener("pointerdown", away, true);
  }, [listOpen]);

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
  // A row of the full list: number, groups and account, the three things the operator checks.
  const listLabel = (udid: string) => {
    const meta = metas?.get(udid);
    const handle = meta?.handle?.trim().replace(/^@+/, "");
    const names = groupNames?.get(udid);
    return {
      number: machineNumberLabel(deviceNumbers?.get(udid) ?? meta?.number),
      groups: names?.length ? names.join(", ") : "Chưa thuộc nhóm",
      account: handle ? `@${handle}` : "Chưa gắn tài khoản",
    };
  };
  const byNumber = (left: string, right: string) =>
    (deviceNumbers?.get(left) ?? metas?.get(left)?.number ?? Number.MAX_SAFE_INTEGER)
    - (deviceNumbers?.get(right) ?? metas?.get(right)?.number ?? Number.MAX_SAFE_INTEGER);
  const closeList = () => {
    setListOpen(false);
    listTrigger.current?.focus();
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
      <div className="selection-bar-scope">
        <button
          type="button"
          ref={listTrigger}
          className="selection-bar-count"
          aria-expanded={listOpen}
          aria-controls="selection-bar-list"
          aria-keyshortcuts="Control+Shift+L"
          title="Xem đủ danh sách máy đã chọn (Ctrl+Shift+L)"
          onClick={() => { setSyncOpen(false); setGroupMenuOpen(false); setListOpen((open) => !open); }}
        >
          <strong>{selected.length} đã chọn</strong>
          <span className="selection-bar-count-detail">
            <span className="visually-hidden"> · </span>{selected.length - hiddenCount} đang thấy · <span className={hiddenCount > 0 ? "selection-bar-hidden" : undefined}>{hiddenCount} ẩn do lọc</span>
            {excluded.length > 0 && <span className="selection-bar-hidden"> · {excluded.length} bị loại</span>}
          </span>
          <ChevronUp size={14} aria-hidden="true" />
        </button>
        {listOpen && (
          <div
            id="selection-bar-list"
            ref={listRef}
            tabIndex={-1}
            className="selection-bar-list"
            role="region"
            aria-label="Danh sách máy đã chọn"
            onKeyDown={(event) => {
              if (event.key !== "Escape") return;
              event.preventDefault();
              event.stopPropagation();
              closeList();
            }}
          >
            <strong>{selected.length} máy nhận lệnh</strong>
            {none && <p>Chưa chọn máy nào. Lệnh ở thanh này chỉ chạy khi có máy đã chọn.</p>}
            <ul>
              {selected.map((device) => device.udid).sort(byNumber).map((udid) => {
                const row = listLabel(udid);
                return (
                  <li key={udid}>
                    <strong>{row.number}</strong>
                    <span>{row.groups}</span>
                    <span>{row.account}</span>
                    {hiddenUdids?.has(udid) && <em>Ẩn do lọc</em>}
                  </li>
                );
              })}
            </ul>
            {excluded.length > 0 && (
              <>
                <strong>Bị loại · không nhận lệnh</strong>
                <ul>
                  {[...excluded].sort((left, right) => byNumber(left.udid, right.udid)).map((entry) => {
                    const row = listLabel(entry.udid);
                    return (
                      <li key={entry.udid}>
                        <strong>{row.number}</strong>
                        <span>{row.account}</span>
                        <span>{entry.reason} · lúc {new Date(entry.at).toLocaleTimeString("vi-VN")}</span>
                      </li>
                    );
                  })}
                </ul>
                <p>Máy bị loại không tự được chọn lại khi rảnh; chọn lại trên lưới nếu vẫn cần.</p>
              </>
            )}
          </div>
        )}
      </div>
      <button
        type="button"
        className="tb-btn primary"
        disabled={none}
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
        disabled={none && !activeSync}
        aria-expanded={syncOpen}
        aria-controls="toolbar-sync-panel"
        onClick={() => { setGroupMenuOpen(false); setListOpen(false); setSyncOpen((open) => !open); }}
        title="Đồng bộ thao tác trên nhóm máy đã chọn"
      >
        <RefreshCcw size={16} aria-hidden="true" />
        {activeSync ? `Đồng bộ · ${activeSync.targetUdids.length} máy` : "Đồng bộ thao tác"}
      </button>
      <button type="button" className="tb-btn" disabled={none} onClick={onRunFlow} title="Mở Flow thiết bị với các máy đã chọn; chạy sau khi xác nhận">
        <Play size={16} aria-hidden="true" />Chạy Flow…
      </button>
      <button type="button" className={`tb-btn ${groupToolsOpen ? "active" : ""}`} disabled={none && !groupToolsOpen} aria-expanded={groupToolsOpen} data-group-tools onClick={onGroupTools} title="Công cụ nhóm: phân phối văn bản/tệp…">
        <SlidersHorizontal size={16} aria-hidden="true" />Công cụ nhóm…
      </button>
      <div ref={groupMenuRef} className="selection-bar-groups">
        <button
          type="button"
          ref={groupTrigger}
          className={`tb-btn ${groupMenuOpen ? "active" : ""}`}
          disabled={none}
          aria-expanded={groupMenuOpen}
          aria-controls="selection-bar-group-list"
          onClick={() => { setSyncOpen(false); setListOpen(false); setGroupMenuOpen((open) => !open); }}
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
      <ExcludedDevicesNotice
        className="selection-bar-excluded"
        excluded={[...excluded].sort((left, right) => byNumber(left.udid, right.udid))}
        label={(udid) => listLabel(udid).number}
        onDismiss={onDismissExcluded}
      />
      <div className="grow" />
      <button type="button" className="tb-btn" onClick={onClear} title="Bỏ chọn mọi máy, kể cả máy bộ lọc đang ẩn">
        <X size={16} aria-hidden="true" />Bỏ chọn
      </button>
    </div>
  );
}
