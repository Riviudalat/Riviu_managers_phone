import type { DeviceInfo } from "../types";
import { IconPhone, IconRefresh } from "./Icons";
import { toastError } from "../toastStore";

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
    <div className="profile-toolbar">
      <button
        type="button"
        className="tb-btn"
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
        className="tb-btn"
        disabled={!any}
        onClick={onStop}
        title="Bỏ chọn"
      >
        Bỏ chọn{any ? ` (${any})` : ""}
      </button>
      <button
        type="button"
        className="tb-btn"
        disabled={!canBatch}
        onClick={async () => {
          try {
            await onInstall();
          } catch (e) {
            toastError("Sửa agent thất bại", e);
          }
        }}
        title={`Cài hoặc khôi phục Riviu Agent cho ${scope}`}
      >
        Khôi phục {scope}
      </button>
      <button
        type="button"
        className={`tb-btn ${syncOn ? "active" : ""}`}
        onClick={onSync}
        title="Đồng bộ thao tác trên nhóm máy đã chọn"
      >
        Đồng bộ{syncOn ? " · Bật" : ""}
      </button>
      <button
        type="button"
        className={`tb-btn ${groupsOpen ? "active" : ""}`}
        onClick={onGroups}
        title="Chia fleet thành nhóm — mỗi máy thuộc đúng một nhóm"
      >
        Nhóm
      </button>
      <button
        type="button"
        className={`tb-btn ${groupToolsOpen ? "active" : ""}`}
        onClick={onGroupTools}
        title="Công cụ nhóm: phân phối văn bản/tệp…"
      >
        Công cụ
      </button>
      <div className="grow" />
      <button type="button" className="tb-btn refresh" onClick={() => void onRefresh()} title="Quét lại thiết bị">
        <IconRefresh size={15} />
      </button>
    </div>
  );
}
