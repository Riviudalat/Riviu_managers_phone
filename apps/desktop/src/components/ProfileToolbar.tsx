import type { DeviceInfo } from "../types";
import { IconPhone, IconRefresh } from "./Icons";
import { toastError } from "../toastStore";
import { FolderKanban, SlidersHorizontal, Wrench, RefreshCcw, ChevronDown } from "lucide-react";

interface Props {
  selected: DeviceInfo[];
  deviceCount: number;
  onStart: () => void | Promise<void>;
  onStop: () => void;
  onInstall: () => void | Promise<void>;
  onSync: () => void;
  onRefresh: () => void | Promise<void>;
  onGroupTools: () => void;
  onGroups: () => void;
  groupsOpen: boolean;
  groupToolsOpen: boolean;
  syncOn: boolean;
}

export function ProfileToolbar({
  selected,
  deviceCount,
  onStart,
  onStop,
  onInstall,
  onSync,
  onRefresh,
  onGroupTools,
  onGroups,
  groupsOpen,
  groupToolsOpen,
  syncOn,
}: Props) {
  const any = selected.length;
  const canBatch = deviceCount > 0;
  const scope = any ? `đã chọn (${any})` : `toàn bộ (${deviceCount})`;

  return (
    <div className="profile-toolbar" role="group" aria-label="Thao tác thiết bị">
      <button
        type="button"
        className="tb-btn primary"
        disabled={!canBatch}
        onClick={async () => {
          try {
            await onStart();
          } catch (e) {
            toastError("Khởi động thất bại", e);
          }
        }}
        title={`Mở luồng xem cho ${scope}`}
      >
        <IconPhone size={16} />
        Mở {scope}
      </button>
      <button
        type="button"
        className={`tb-btn ${syncOn ? "active" : ""}`}
        aria-pressed={syncOn}
        onClick={onSync}
        title="Đồng bộ thao tác trên nhóm máy đã chọn"
      >
        <RefreshCcw size={16} aria-hidden="true" />Đồng bộ{syncOn ? " · Bật" : ""}
      </button>
      <button
        type="button"
        className={`tb-btn ${groupsOpen ? "active" : ""}`}
        aria-expanded={groupsOpen}
        onClick={onGroups}
        title="Chia fleet thành nhóm — mỗi máy thuộc đúng một nhóm"
      >
        <FolderKanban size={16} aria-hidden="true" />Nhóm
      </button>
      <button
        type="button"
        className={`tb-btn ${groupToolsOpen ? "active" : ""}`}
        aria-expanded={groupToolsOpen}
        data-group-tools
        onClick={onGroupTools}
        title="Công cụ nhóm: phân phối văn bản/tệp…"
      >
        <SlidersHorizontal size={16} aria-hidden="true" />Công cụ
      </button>
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
          <button type="button" className="tb-btn" disabled={!canBatch}
            onClick={async (event) => {
              event.currentTarget.closest("details")?.removeAttribute("open");
              try { await onInstall(); } catch (e) { toastError("Sửa agent thất bại", e); }
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
