import { useEffect, useRef, useState, type ReactElement } from "react";
import { deviceModelOsLabel } from "../types";
import type { DeviceInfo, HardwareKey } from "../types";
import {
  backupDevice,
  deviceKey,
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
  IconBack,
  IconBell,
  IconCamera,
  IconClose,
  IconHome,
  IconPower,
  IconRecents,
  IconVolumeDown,
  IconVolumeUp,
} from "./Icons";

interface Props {
  device: DeviceInfo;
  onClose: () => void;
  groupUdids: string[];
  groupMode: boolean;
}

type KeySpec = {
  key: HardwareKey;
  title: string;
  androidOnly?: boolean;
  Icon: (props: { size?: number }) => ReactElement;
};

const HARDWARE_KEYS: KeySpec[] = [
  { key: "back", title: "Back", androidOnly: true, Icon: IconBack },
  { key: "home", title: "Home", Icon: IconHome },
  { key: "recents", title: "Recents", androidOnly: true, Icon: IconRecents },
  { key: "volumeUp", title: "Tăng âm lượng", androidOnly: true, Icon: IconVolumeUp },
  { key: "volumeDown", title: "Giảm âm lượng", androidOnly: true, Icon: IconVolumeDown },
  { key: "power", title: "Khóa màn hình", androidOnly: true, Icon: IconPower },
  { key: "notification", title: "Thông báo", androidOnly: true, Icon: IconBell },
];

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
  const x = ((clientX - rect.left) / rect.width) * nw;
  const y = ((clientY - rect.top) / rect.height) * nh;
  return {
    x: Math.max(0, Math.min(nw, x)),
    y: Math.max(0, Math.min(nh, y)),
  };
}

export function FocusStream({ device, onClose, groupUdids, groupMode }: Props) {
  useHydratedDeviceFrame(device.udid, latestFrame);
  const frame = useDeviceFrame(device.udid) ?? peekFrame(device.udid) ?? null;
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const imgRef = useRef<HTMLImageElement>(null);
  const drag = useRef<{ x: number; y: number } | null>(null);
  const targets = groupMode && groupUdids.length > 1 ? groupUdids : [device.udid];
  const isIos = device.platform === "ios";

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

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

  const pressKey = async (key: HardwareKey) => {
    try {
      if (targets.length > 1) {
        await groupInput({ udids: targets, kind: "key", key });
      } else {
        await deviceKey(device.udid, key);
      }
    } catch (e) {
      toastError("Không bấm được phím", e);
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

  const IOS_ONLY_BACKUP = "Backup dùng Mobilebackup2 — chỉ có trên iPhone";
  const IOS_ONLY_RESTORE = "Phục hồi backup Mobilebackup2 — chỉ có trên iPhone";

  const backup = async () => {
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
    <div
      className="focus-overlay"
      role="dialog"
      aria-modal="true"
      aria-label={`Điều khiển ${device.name}`}
      onClick={onClose}
    >
      <div
        className="focus-stage"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="focus-phone">
          <header className="focus-phone-head">
            <div className="title">
              <strong>{device.name}</strong>
              <span className="hint">{deviceModelOsLabel(device)}</span>
            </div>
            <button type="button" className="close" title="Đóng" aria-label="Đóng" onClick={onClose}>
              <IconClose size={16} />
            </button>
          </header>
          <div className="focus-phone-screen">
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
          <div className="focus-phone-tools">
            <label>
              Gõ chữ
              <input
                value={text}
                onChange={(e) => setText(e.target.value)}
                placeholder="Nội dung cần gõ…"
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    void sendKeys();
                  }
                }}
              />
            </label>
            <button type="button" className="primary" onClick={() => void sendKeys()}>
              Gửi{targets.length > 1 ? ` · ${targets.length}` : ""}
            </button>
            <div className="focus-phone-extra">
              <button type="button" onClick={() => void reboot()}>
                Khởi động lại
              </button>
              <button type="button" disabled={!isIos} onClick={() => void backup()}>
                Backup
              </button>
              <button
                type="button"
                className="danger"
                disabled={!isIos}
                onClick={() => void restore()}
              >
                Restore
              </button>
            </div>
            <p className="hint">
              {busy ? "Đang gửi lệnh…" : "Click hoặc kéo trên màn hình để điều khiển."}
            </p>
            {groupMode && targets.length > 1 && (
              <p className="hint">Đồng bộ nhóm: {targets.length} máy nhận cùng thao tác.</p>
            )}
          </div>
        </div>

        <nav className="focus-keys" aria-label="Phím chức năng">
          {HARDWARE_KEYS.map(({ key, title, androidOnly, Icon }) => {
            const disabled = Boolean(androidOnly && isIos);
            return (
              <button
                key={key}
                type="button"
                disabled={disabled}
                title={
                  disabled
                    ? `${title} — chỉ có trên Android`
                    : title
                }
                onClick={() => void pressKey(key)}
              >
                <Icon size={18} />
              </button>
            );
          })}
          <button type="button" title="Chụp màn hình" onClick={() => void capture()}>
            <IconCamera size={18} />
          </button>
        </nav>
      </div>
    </div>
  );
}
