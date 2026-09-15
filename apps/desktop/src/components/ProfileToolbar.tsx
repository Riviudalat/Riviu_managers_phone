import type { DeviceInfo, DeviceMeta } from "../types";
import { useRef, useState } from "react";
import { tileName } from "../deviceNaming";
import { GroupSyncSection } from "./settings/GroupSyncSection";
import { IconPhone, IconRefresh } from "./Icons";
import { toastError } from "../toastStore";
import { FolderKanban, SlidersHorizontal, Wrench, RefreshCcw, ChevronDown } from "lucide-react";

interface Props {
  controlCenter?: string | null;
  onControlCenter?: (udid: string | null) => void;
  deviceNumbers?: Map<string, number>;
  metas?: Map<string, DeviceMeta>;
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
  controlCenter,
  onControlCenter,
  deviceNumbers,
  metas,
}: Props) {
  const [syncOpen, setSyncOpen] = useState(false);
  const syncTrigger = useRef<HTMLButtonElement>(null);
  const labelFor = (device: DeviceInfo) => {
    const meta = metas?.get(device.udid);
    const number = deviceNumbers?.get(device.udid) ?? meta?.number;
    return `${number ? `Máy ${number} · ` : ""}${tileName(device, meta)}${meta?.handle ? ` · @${meta.handle.replace(/^@+/, "")}` : ""}`;
  };
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
        ref={syncTrigger}
        aria-expanded={syncOpen}
        aria-controls="toolbar-sync-panel"
        onClick={() => setSyncOpen(open => !open)}
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
      {syncOpen && <section id="toolbar-sync-panel" className="toolbar-sync-panel" aria-label="Điều khiển đồng bộ" onKeyDown={event=>{
        if(event.key==="Escape") {event.preventDefault();event.stopPropagation();setSyncOpen(false);syncTrigger.current?.focus();}
      }}>
        <div className="toolbar-sync-heading"><div><strong>Đồng bộ thao tác</strong><span>{syncOn?"Đang bật":"Đang tắt"} · {selected.length} máy đã chọn</span></div>
          <button type="button" className="tb-btn" onClick={()=>{setSyncOpen(false);syncTrigger.current?.focus();}}>Đóng đồng bộ</button>
        </div>
        <p className="hint">Chạm, vuốt, gõ và phím trên máy chính sẽ gửi tới nhóm đã chọn. Máy đang bận sẽ báo lỗi riêng.</p>
        <label className="toolbar-sync-master">Máy chính <select value={controlCenter??""} onChange={e=>onControlCenter?.(e.target.value||null)}>
          <option value="">Máy đang mở</option>{controlCenter&&!selected.some(d=>d.udid===controlCenter)&&<option value={controlCenter}>Máy chính nằm ngoài nhóm — chọn lại</option>}{selected.map(d=><option key={d.udid} value={d.udid}>{labelFor(d)}</option>)}
        </select></label>
        <div className="toolbar-sync-targets" aria-label="Máy nhận thao tác">{selected.map(device=><span key={device.udid}>{labelFor(device)}</span>)}</div>
        <button type="button" className={`tb-btn ${syncOn?"active":"primary"}`} aria-pressed={syncOn} disabled={!syncOn&&(selected.length<2||Boolean(controlCenter&&!selected.some(d=>d.udid===controlCenter)))} onClick={onSync}>{syncOn?"Tắt đồng bộ thao tác":"Bật đồng bộ thao tác"}</button>
        {selected.length<2&&<p>Chọn ít nhất hai máy để bật đồng bộ.</p>}
        <details className="toolbar-sync-options"><summary>Độ trễ và độ lệch thao tác</summary><GroupSyncSection /></details>
      </section>}
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
