import { useRef, useState } from "react";
import { ChevronDown, FolderKanban, RefreshCcw, SlidersHorizontal, Wrench } from "lucide-react";

import type { ActiveGroupSync, GroupSyncReadiness } from "../groupSync";
import type { DeviceInfo, DeviceMeta } from "../types";
import { tileName } from "../deviceNaming";
import { toastError } from "../toastStore";
import { IconPhone, IconRefresh } from "./Icons";
import { GroupSyncSection } from "./settings/GroupSyncSection";

interface Props {
  activeSync: ActiveGroupSync | null;
  readiness: GroupSyncReadiness | null;
  resolvedMasterUdid: string | null;
  onMasterChange: (udid: string | null) => void;
  onEnableSync: (masterUdid: string) => void;
  onDisableSync: () => void;
  deviceNumbers?: Map<string, number>;
  metas?: Map<string, DeviceMeta>;
  selected: DeviceInfo[];
  deviceCount: number;
  onStart: () => void | Promise<void>;
  onStop: () => void;
  onInstall: () => void | Promise<void>;
  onRefresh: () => void | Promise<void>;
  onGroupTools: () => void;
  onGroups: () => void;
  groupsOpen: boolean;
  groupToolsOpen: boolean;
}

export function ProfileToolbar({
  activeSync,
  readiness,
  resolvedMasterUdid,
  onMasterChange,
  onEnableSync,
  onDisableSync,
  selected,
  deviceCount,
  onStart,
  onStop,
  onInstall,
  onRefresh,
  onGroupTools,
  onGroups,
  groupsOpen,
  groupToolsOpen,
  deviceNumbers,
  metas,
}: Props) {
  const [syncOpen, setSyncOpen] = useState(false);
  const syncTrigger = useRef<HTMLButtonElement>(null);
  const ready = new Set(readiness?.readyUdids ?? []);
  const failures = readiness?.failures ?? {};
  const labelFor = (device: DeviceInfo) => {
    const meta = metas?.get(device.udid);
    const number = deviceNumbers?.get(device.udid) ?? meta?.number;
    return `${number ? `Máy ${number} · ` : ""}${tileName(device, meta)}${meta?.handle ? ` · @${meta.handle.replace(/^@+/, "")}` : ""}`;
  };
  const any = selected.length;
  const canBatch = deviceCount > 0;
  const scope = any ? `đã chọn (${any})` : `toàn bộ (${deviceCount})`;
  const masterValid = Boolean(
    resolvedMasterUdid && selected.some((device) => device.udid === resolvedMasterUdid),
  );
  const disconnected = selected.filter((device) => device.status === "disconnected");
  const enableReason = selected.length < 2
    ? "Chọn ít nhất hai máy để bật đồng bộ."
    : !masterValid
      ? "Chọn lại một máy chính trong phạm vi."
      : disconnected.length > 0
        ? `${disconnected.map(labelFor).join(", ")} đang ngoại tuyến.`
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

  return (
    <div className="profile-toolbar" role="group" aria-label="Thao tác thiết bị">
      <button
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
      </button>
      <button
        type="button"
        className={`tb-btn ${activeSync ? "active" : ""}`}
        ref={syncTrigger}
        aria-expanded={syncOpen}
        aria-controls="toolbar-sync-panel"
        onClick={() => setSyncOpen((open) => !open)}
        title="Đồng bộ thao tác trên nhóm máy đã chọn"
      >
        <RefreshCcw size={16} aria-hidden="true" />
        {activeSync ? `Đồng bộ · ${activeSync.targetUdids.length} máy` : "Đồng bộ"}
      </button>
      <button type="button" className={`tb-btn ${groupsOpen ? "active" : ""}`} aria-expanded={groupsOpen} onClick={onGroups} title="Chia fleet thành nhóm — mỗi máy thuộc đúng một nhóm">
        <FolderKanban size={16} aria-hidden="true" />Nhóm
      </button>
      <button type="button" className={`tb-btn ${groupToolsOpen ? "active" : ""}`} aria-expanded={groupToolsOpen} data-group-tools onClick={onGroupTools} title="Công cụ nhóm: phân phối văn bản/tệp…">
        <SlidersHorizontal size={16} aria-hidden="true" />Công cụ
      </button>
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
                ? "Ngoại tuyến"
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
      <span className="toolbar-scope">{any ? `${any} máy đã chọn` : `${deviceCount} máy trong hệ thống`}</span>
      {any > 0 && <button type="button" className="tb-btn" onClick={onStop} title="Bỏ chọn">Bỏ chọn ({any})</button>}
      <div className="grow" />
      <details className="toolbar-maintenance" onKeyDown={(event) => {
        if (event.key !== "Escape") return;
        event.preventDefault();
        event.currentTarget.removeAttribute("open");
        event.currentTarget.querySelector("summary")?.focus();
      }}>
        <summary tabIndex={0}><Wrench size={15} aria-hidden="true" />Bảo trì<ChevronDown size={14} aria-hidden="true" /></summary>
        <div className="toolbar-maintenance-menu">
          <strong>Bảo trì thiết bị</strong>
          <span>{any ? `${any} máy đã chọn` : "Các máy đang kết nối"}</span>
          <button type="button" className="tb-btn" disabled={!canBatch} onClick={async (event) => {
            event.currentTarget.closest("details")?.removeAttribute("open");
            try { await onInstall(); } catch (error) { toastError("Sửa agent thất bại", error); }
          }} title={`Cài hoặc khôi phục Riviu Agent cho ${scope}`}>
            <Wrench size={16} aria-hidden="true" />Sửa Riviu Agent
          </button>
        </div>
      </details>
      <button type="button" className="tb-btn refresh" onClick={() => void onRefresh()} title="Quét lại thiết bị">
        <IconRefresh size={15} />
      </button>
    </div>
  );
}
