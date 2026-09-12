import { X } from "lucide-react";
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
      </aside>
    </div>
  );
}
