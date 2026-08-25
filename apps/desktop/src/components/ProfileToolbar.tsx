import type { DeviceInfo } from "../types";
import { IconChat, IconHeart, IconPhone, IconRefresh } from "./Icons";
import { toastError } from "../toastStore";

interface Props {
  selected: DeviceInfo[];
  onStart: () => void | Promise<void>;
  onStop: () => void;
  onInstall: () => void | Promise<void>;
  onSync: () => void;
  onRefresh: () => void | Promise<void>;
  onNurture: () => void;
  nurtureOpen: boolean;
  onInteraction: () => void;
  interactionOpen: boolean;
  onGroupTools: () => void;
  onGroups: () => void;
  groupsOpen: boolean;
  groupToolsOpen: boolean;
  syncOn: boolean;
}

export function ProfileToolbar({
  selected,
  onStart,
  onStop,
  onInstall,
  onSync,
  onRefresh,
  onNurture,
  nurtureOpen,
  onInteraction,
  interactionOpen,
  onGroupTools,
  onGroups,
  groupsOpen,
  groupToolsOpen,
  syncOn,
}: Props) {
  const startable = selected.filter(
    (d) =>
      d.status === "connected" ||
      d.status === "ready" ||
      d.status === "preparing" ||
      d.status === "busy" ||
      d.status === "error",
  );
  const any = selected.length;
  const canBatch = true;

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
        title="Prepare / start stream (selected hoặc tất cả)"
      >
        <IconPhone size={16} />
        Start{startable.length ? `(${startable.length})` : any ? `(${any})` : ""}
      </button>
      <button
        type="button"
        className="tb-btn"
        disabled={!any}
        onClick={onStop}
        title="Bỏ chọn"
      >
        Bỏ chọn{any ? `(${any})` : ""}
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
        title="Cài / re-sign agent"
      >
        Agent{any ? `(${any})` : ""}
      </button>
      <button
        type="button"
        className={`tb-btn ${syncOn ? "active" : ""}`}
        onClick={onSync}
        title="Group sync"
      >
        Sync{syncOn ? " · ON" : ""}
      </button>
      <button
        type="button"
        className={`tb-btn ${nurtureOpen ? "active" : ""}`}
        onClick={onNurture}
        title="Nuôi TikTok"
      >
        <IconHeart size={15} />
        Nuôi TT
      </button>
      <button
        type="button"
        className={`tb-btn ${interactionOpen ? "active" : ""}`}
        onClick={onInteraction}
        title="Tương tác comment theo link TikTok"
      >
        <IconChat size={15} />
        Tương tác
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
