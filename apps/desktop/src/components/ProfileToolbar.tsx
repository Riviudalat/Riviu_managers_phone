import { ChevronDown, FolderKanban, SlidersHorizontal, Wrench } from "lucide-react";

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
      <button type="button" className={`tb-btn ${groupsOpen ? "active" : ""}`} aria-expanded={groupsOpen} onClick={onGroups} title="Chia fleet thành nhóm — mỗi máy thuộc đúng một nhóm">
        <FolderKanban size={16} aria-hidden="true" />Nhóm
      </button>
      {!any && <button type="button" className={`tb-btn ${groupToolsOpen ? "active" : ""}`} aria-expanded={groupToolsOpen} data-group-tools onClick={onGroupTools} title="Công cụ nhóm: phân phối văn bản/tệp…">
        <SlidersHorizontal size={16} aria-hidden="true" />Công cụ
      </button>}
      <span className="toolbar-scope">{any ? `${any} máy đã chọn · thao tác ở thanh dưới` : `${deviceCount} máy trong hệ thống`}</span>
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
          <button type="button" className="tb-btn" disabled={!canBatch || !onHelperMaintenance} onClick={async (event) => {
            event.currentTarget.closest("details")?.removeAttribute("open");
            try { await onHelperMaintenance?.(); } catch (error) { toastError("Khôi phục helper thất bại", error); }
          }} title={any ? "Chuẩn bị và xác nhận khôi phục helper trên các máy Android đã chọn" : "Chuẩn bị và xác nhận khôi phục helper trên các máy Android đang báo cần khôi phục helper"}>
            <Wrench size={16} aria-hidden="true" />Khôi phục helper
          </button>
          <button type="button" className="tb-btn" disabled={!canBatch || !onQuarantineMaintenance} onClick={async (event) => {
            event.currentTarget.closest("details")?.removeAttribute("open");
            try { await onQuarantineMaintenance?.(); } catch (error) { toastError("Khôi phục phiên điều khiển thất bại", error); }
          }} title="Khôi phục phiên TikTok được giữ lại trên các máy Android đang kết nối trong phạm vi; kế hoạch đã gửi chỉ được tra cứu">
            <Wrench size={16} aria-hidden="true" />Khôi phục phiên điều khiển
          </button>
        </div>
      </details>
      <button type="button" className="tb-btn refresh" onClick={() => void onRefresh()} title="Quét lại thiết bị">
        <IconRefresh size={15} />
      </button>
    </div>
  );
}
