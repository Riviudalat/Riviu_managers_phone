import { useRef, useState } from "react";
import { deviceModelOsLabel } from "../types";
import type { DeviceInfo } from "../types";
import {
  backupDevice,
  deviceHome,
  deviceSwipe,
  deviceTap,
  deviceTypeText,
  groupInput,
  latestFrame,
  rebootDevice,
  restoreDevice,
  screenshot,
} from "../api";
import { pickDirectory } from "../pickFile";
import { peekFrame, useDeviceFrame, useHydratedDeviceFrame } from "../frameStore";
import { requestConfirm } from "../confirmStore";
import { pushToast, toastError } from "../toastStore";
import {
  IconCamera,
  IconChevronLeft,
  IconChevronRight,
  IconClose,
  IconDownload,
  IconHome,
  IconPower,
  IconUpload,
} from "./Icons";

interface Props {
  device: DeviceInfo;
  onClose: () => void;
  groupUdids: string[];
  groupMode: boolean;
}

function mapToDevice(
  img: HTMLImageElement,
  clientX: number,
  clientY: number,
): { x: number; y: number } {
  const rect = img.getBoundingClientRect();
  const nw = img.naturalWidth || 375;
  const nh = img.naturalHeight || 667;
  if (rect.width <= 0 || rect.height <= 0) {
    return { x: nw / 2, y: nh / 2 };
  }
  // <img> box matches painted pixels (height:100%; width:auto; object-fit:contain)
  const x = ((clientX - rect.left) / rect.width) * nw;
  const y = ((clientY - rect.top) / rect.height) * nh;
  return {
    x: Math.max(0, Math.min(nw, x)),
    y: Math.max(0, Math.min(nh, y)),
  };
}

/**
 * Device control dock. Sits beside the fleet grid as a real layout column
 * rather than a near-fullscreen overlay, so the operator keeps sight of the
 * other devices while driving one. Collapses to a screen-only strip.
 */
export function FocusStream({ device, onClose, groupUdids, groupMode }: Props) {
  useHydratedDeviceFrame(device.udid, latestFrame);
  const frame = useDeviceFrame(device.udid) ?? peekFrame(device.udid) ?? null;
  const [text, setText] = useState("");
  const [compact, setCompact] = useState(false);
  const [busy, setBusy] = useState(false);
  const imgRef = useRef<HTMLImageElement>(null);
  const drag = useRef<{ x: number; y: number } | null>(null);
  const targets = groupMode && groupUdids.length > 1 ? groupUdids : [device.udid];

  const runGesture = async (start: { x: number; y: number }, end: { x: number; y: number }) => {
    const img = imgRef.current;
    if (!img) return;
    const iw = img.naturalWidth || 375;
    const ih = img.naturalHeight || 667;
    const dist = Math.hypot(end.x - start.x, end.y - start.y);
    setBusy(true);
    try {
      if (dist < 10) {
        if (targets.length > 1) {
          await groupInput({
            udids: targets,
            kind: "tap",
            x: end.x,
            y: end.y,
            imageW: iw,
            imageH: ih,
          });
        } else {
          await deviceTap(device.udid, end.x, end.y, iw, ih);
        }
      } else if (targets.length > 1) {
        await groupInput({
          udids: targets,
          kind: "swipe",
          x: start.x,
          y: start.y,
          toX: end.x,
          toY: end.y,
          imageW: iw,
          imageH: ih,
        });
      } else {
        await deviceSwipe(device.udid, start.x, start.y, end.x, end.y, iw, ih);
      }
    } catch (e) {
      toastError("Điều khiển thất bại", e);
    } finally {
      setBusy(false);
    }
  };

  const goHome = async () => {
    try {
      await deviceHome(device.udid);
    } catch (e) {
      toastError("Không về được màn hình chính", e);
    }
  };

  const capture = async () => {
    try {
      pushToast("ok", "Đã chụp màn hình", await screenshot(device.udid));
    } catch (e) {
      toastError("Chụp màn hình thất bại", e);
    }
  };

  const reboot = async () => {
    const proceed = await requestConfirm({
      title: `Khởi động lại ${device.name}?`,
      message: "Thiết bị sẽ ngắt kết nối vài phút và stream dừng cho tới khi khởi động xong.",
      confirmLabel: "Khởi động lại",
      danger: true,
    });
    if (!proceed) return;
    try {
      await rebootDevice(device.udid);
      pushToast("info", "Đang khởi động lại", device.name);
    } catch (e) {
      toastError("Khởi động lại thất bại", e);
    }
  };

  // Mobilebackup2 is an iOS service. There is no Android analogue: `adb backup`
  // is deprecated and removed on modern Android, so this is a permanent platform
  // limit rather than a feature waiting to be written. The buttons stay visible but
  // disabled — they sit in a fixed five-icon row, and hiding two of them reflows
  // that row per device, giving an operator dragging across the fleet a moving
  // target.
  const isIos = device.platform === "ios";
  const IOS_ONLY_BACKUP = "Backup dùng Mobilebackup2 — chỉ có trên iPhone";
  const IOS_ONLY_RESTORE = "Phục hồi backup Mobilebackup2 — chỉ có trên iPhone";

  const backup = async () => {
    // `disabled` is an affordance, not a guard: a keyboard or programmatic path
    // still reaches this.
    if (!isIos) {
      pushToast("info", "Không hỗ trợ", IOS_ONLY_BACKUP);
      return;
    }
    const dir = await pickDirectory("Chọn thư mục lưu backup");
    if (!dir) return;
    pushToast("info", "Đang backup…", `${device.name} — có thể mất vài phút.`);
    try {
      await backupDevice(device.udid, dir);
      pushToast("ok", "Backup xong", dir);
    } catch (e) {
      toastError("Backup thất bại", e);
    }
  };

  const restore = async () => {
    if (!isIos) {
      pushToast("info", "Không hỗ trợ", IOS_ONLY_RESTORE);
      return;
    }
    const dir = await pickDirectory("Chọn thư mục backup để phục hồi");
    if (!dir) return;
    const proceed = await requestConfirm({
      title: `Phục hồi ${device.name} từ backup?`,
      message: "Toàn bộ dữ liệu hiện tại trên thiết bị sẽ bị ghi đè và máy sẽ khởi động lại.",
      confirmLabel: "Ghi đè & phục hồi",
      danger: true,
    });
    if (!proceed) return;
    pushToast("info", "Đang phục hồi…", device.name);
    try {
      await restoreDevice(device.udid, dir);
      pushToast("ok", "Đã phục hồi", "Thiết bị sẽ khởi động lại.");
    } catch (e) {
      toastError("Phục hồi thất bại", e);
    }
  };

  const sendKeys = async () => {
    try {
      if (targets.length > 1) {
        await groupInput({ udids: targets, kind: "type", text });
      } else {
        await deviceTypeText(device.udid, text);
      }
      pushToast("ok", "Đã gửi phím", `${text.length} ký tự · ${targets.length} máy`);
    } catch (e) {
      toastError("Gửi phím thất bại", e);
    }
  };

  return (
    <aside
      className={`focus-dock ${compact ? "compact" : ""}`}
      aria-label={`Điều khiển ${device.name}`}
    >
      <header className="focus-dock-head">
        <div className="title">
          <strong>{device.name}</strong>
          <span className="hint">{deviceModelOsLabel(device)}</span>
        </div>
        <button
          type="button"
          title={compact ? "Mở rộng" : "Thu gọn"}
          aria-label={compact ? "Mở rộng bảng điều khiển" : "Thu gọn bảng điều khiển"}
          onClick={() => setCompact((v) => !v)}
        >
          {compact ? <IconChevronLeft size={14} /> : <IconChevronRight size={14} />}
        </button>
        <button type="button" className="close" title="Đóng" aria-label="Đóng" onClick={onClose}>
          <IconClose size={14} />
        </button>
      </header>

      <div className="focus-dock-screen">
        <div className="focus-screen">
          {frame ? (
            <img
              ref={imgRef}
              src={`data:image/jpeg;base64,${frame}`}
              alt={device.name}
              draggable={false}
              className="focus-touch"
              onPointerDown={(e) => {
                if (e.button !== 0 || !imgRef.current) return;
                e.preventDefault();
                drag.current = mapToDevice(imgRef.current, e.clientX, e.clientY);
                e.currentTarget.setPointerCapture(e.pointerId);
              }}
              onPointerUp={async (e) => {
                if (e.button !== 0 || !drag.current || !imgRef.current) {
                  drag.current = null;
                  return;
                }
                e.preventDefault();
                const start = drag.current;
                const end = mapToDevice(imgRef.current, e.clientX, e.clientY);
                drag.current = null;
                await runGesture(start, end);
              }}
              onPointerCancel={() => {
                drag.current = null;
              }}
            />
          ) : (
            <div className="screen-empty">Đang chờ stream…</div>
          )}
        </div>
      </div>

      <nav className="focus-dock-nav" aria-label="Thao tác thiết bị">
        <button type="button" title="Về màn hình chính" onClick={() => void goHome()}>
          <IconHome size={17} />
        </button>
        <button type="button" title="Chụp màn hình" onClick={() => void capture()}>
          <IconCamera size={17} />
        </button>
        <button type="button" title="Khởi động lại" onClick={() => void reboot()}>
          <IconPower size={17} />
        </button>
        <button
          type="button"
          disabled={!isIos}
          title={
            isIos ? "Backup thiết bị (Mobilebackup2 — có thể mất vài phút)" : IOS_ONLY_BACKUP
          }
          onClick={() => void backup()}
        >
          <IconDownload size={17} />
        </button>
        <button
          type="button"
          className="danger"
          disabled={!isIos}
          title={
            isIos
              ? "Phục hồi từ backup (ghi đè dữ liệu + khởi động lại)"
              : IOS_ONLY_RESTORE
          }
          onClick={() => void restore()}
        >
          <IconUpload size={17} />
        </button>
      </nav>

      {!compact && (
        <div className="focus-dock-tools">
          <label>
            Gõ chữ
            <input
              value={text}
              onChange={(e) => setText(e.target.value)}
              placeholder="Nội dung cần gõ…"
            />
          </label>
          <button type="button" className="primary" onClick={() => void sendKeys()}>
            Gửi phím{targets.length > 1 ? ` · ${targets.length} máy` : ""}
          </button>
          <p className="hint">
            {busy ? "Đang gửi lệnh…" : "Click hoặc kéo trên màn hình để điều khiển."}
          </p>
          {groupMode && targets.length > 1 && (
            <p className="hint">Đồng bộ nhóm: {targets.length} máy nhận cùng thao tác.</p>
          )}
          <p className="hint mono">{device.udid}</p>
        </div>
      )}
    </aside>
  );
}
