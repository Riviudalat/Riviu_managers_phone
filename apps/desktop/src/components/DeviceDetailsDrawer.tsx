import { X } from "lucide-react";
import { useState } from "react";
import { deviceActionCapabilities } from "../api";
import { describeError } from "../describeError";
import type { DeviceActionCapabilities } from "../generated-ipc";
import { useModalFocus } from "./useModalFocus";
import { StatusChip } from "./WorkspacePrimitives";

import { deviceWorkOwnerLabel } from "../deviceWork";
import { deviceOsLabel } from "../types";
import type { DeviceInfo, DeviceWorkOwner } from "../types";

export function DeviceDetailsDrawer({
  device,
  machineLabel,
  currentOwner,
  ownerReadFailed,
  onClose,
}: {
  device: DeviceInfo;
  machineLabel: string;
  currentOwner: DeviceWorkOwner | null;
  ownerReadFailed: boolean;
  onClose: () => void;
}) {
  const dialogRef = useModalFocus<HTMLElement>(onClose);
  const [capabilities, setCapabilities] = useState<DeviceActionCapabilities | null>(null);
  const [capabilityError, setCapabilityError] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  const readCapabilities = async () => {
    setChecking(true); setCapabilityError(null);
    try { setCapabilities(await deviceActionCapabilities(device.udid)); }
    catch (error) { setCapabilityError(describeError(error)); }
    finally { setChecking(false); }
  };
  const stateLabels = { measured: "Đã đo", runtimeProofRequired: "Cần chứng minh trong phiên", unsupported: "Chưa hỗ trợ", deviceNotReady: "Máy chưa sẵn sàng" };
  const actionLabels: Record<string, string> = { feed: "Lướt feed", search: "Tìm kiếm", photo: "Đăng ảnh", video: "Đăng video", sound: "Chọn nhạc", like: "Thích", save: "Lưu", follow: "Theo dõi tài khoản đích", feedFollow: "Theo dõi trong Nuôi", mentionReply: "Trả lời và tag" };

  const ownerLabel = ownerReadFailed
    ? "Chưa đọc được"
    : currentOwner
      ? deviceWorkOwnerLabel(currentOwner)
      : "Đang rảnh";

  return (
    <div className="device-detail-backdrop" onClick={onClose}>
      <aside
        ref={dialogRef}
        tabIndex={-1}
        className="device-detail-drawer"
        role="dialog"
        aria-modal="true"
        aria-labelledby="device-detail-title"
        onClick={(event) => event.stopPropagation()}
      >
        <header>
          <div>
            <p className="device-detail-kicker">Thông tin thiết bị</p>
            <h2 id="device-detail-title">Chi tiết {machineLabel}</h2>
            <span>{device.name}</span>
          </div>
          <button
            type="button"
            className="icon-button"
            aria-label="Đóng chi tiết thiết bị"
            title="Đóng"
            onClick={onClose}
          >
            <X size={18} />
          </button>
        </header>

        <dl className="device-detail-list">
          <div><dt>Tác vụ hiện tại</dt><dd><StatusChip tone={ownerReadFailed ? "warning" : currentOwner ? "info" : "success"}>{ownerLabel}</StatusChip></dd></div>
          <div><dt>Dòng máy</dt><dd>{device.model}</dd></div>
          <div><dt>Hệ điều hành</dt><dd>{deviceOsLabel(device)}</dd></div>
          <div><dt>Serial / UDID</dt><dd className="mono">{device.udid}</dd></div>
          <div><dt>Kết nối</dt><dd>{device.connection.toUpperCase()}</dd></div>
          <div><dt>Trạng thái gốc</dt><dd className="mono">{device.status}</dd></div>
          <div><dt>Luồng hình</dt><dd>{device.tileStreamState ?? "Chưa có dữ liệu"}</dd></div>
        </dl>

        <section className="device-detail-evidence" aria-label="Lỗi và bằng chứng gần nhất">
          <h3>Lỗi và bằng chứng gần nhất</h3>
          {device.lastError ? <pre>{device.lastError}</pre> : <p>Chưa ghi nhận lỗi.</p>}
        </section>
        <section className="device-detail-evidence" aria-label="Khả năng thiết bị">
          <h3>Khả năng thiết bị</h3>
          <button type="button" disabled={checking} onClick={() => void readCapabilities()}>{checking ? "Đang đọc…" : "Kiểm tra khả năng"}</button>
          {capabilityError && <p role="alert">{capabilityError}</p>}
          {capabilities?.udid === device.udid && <>
            <p>{capabilities.package} · {capabilities.version} · {capabilities.locale}</p>
            <dl className="device-detail-list">{capabilities.actions.map(action => <div key={action.action}>
              <dt>{actionLabels[action.action] ?? action.action}</dt><dd>{stateLabels[action.state]}<small style={{ display: "block" }}>{action.reason}</small></dd>
            </div>)}</dl>
          </>}
        </section>
      </aside>
    </div>
  );
}
