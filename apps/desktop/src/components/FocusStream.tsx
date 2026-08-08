import { useRef, useState } from "react";
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
import {
  IconBack,
  IconCamera,
  IconClose,
  IconHome,
  IconPin,
  IconPower,
  IconVolume,
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

export function FocusStream({ device, onClose, groupUdids, groupMode }: Props) {
  useHydratedDeviceFrame(device.udid, latestFrame);
  const frame = useDeviceFrame(device.udid) ?? peekFrame(device.udid) ?? null;
  const [text, setText] = useState("");
  const [pinned, setPinned] = useState(true);
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
      window.alert(`Điều khiển thất bại:\n${e}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="focus-layer" aria-label="Focus stream">
      {!pinned && <div className="focus-backdrop" onClick={onClose} />}
      <div className="focus-panel floating">
        <div className="focus-titlebar">
          <div className="title">
            <strong>{device.name}</strong>
            <span className="hint">
              {device.model} · iOS {device.iosVersion}
              {busy ? " · đang gửi…" : " · click/kéo để điều khiển"}
            </span>
          </div>
          <div className="win-btns">
            <button
              type="button"
              className={pinned ? "active" : ""}
              title={pinned ? "Bỏ ghim" : "Ghim"}
              onClick={() => setPinned((v) => !v)}
            >
              <IconPin size={14} />
            </button>
            <button type="button" className="close" title="Đóng" onClick={onClose}>
              <IconClose size={14} />
            </button>
          </div>
        </div>

        <div className="focus-body">
          <nav className="focus-nav">
            <button
              type="button"
              title="Home"
              onClick={async () => {
                try {
                  await deviceHome(device.udid);
                } catch (e) {
                  window.alert(String(e));
                }
              }}
            >
              <IconHome size={18} />
            </button>
            <button type="button" title="Back (iOS hạn chế)" disabled>
              <IconBack size={18} />
            </button>
            <button
              type="button"
              title="Screenshot"
              onClick={async () => {
                try {
                  alert(`Đã lưu: ${await screenshot(device.udid)}`);
                } catch (e) {
                  window.alert(String(e));
                }
              }}
            >
              <IconCamera size={18} />
            </button>
            <button type="button" title="Volume (chưa hỗ trợ)" disabled>
              <IconVolume size={18} />
            </button>
            <button
              type="button"
              title="Reboot"
              onClick={async () => {
                try {
                  await rebootDevice(device.udid);
                } catch (e) {
                  window.alert(String(e));
                }
              }}
            >
              <IconPower size={18} />
            </button>
            <button
              type="button"
              title="Backup thiết bị (Mobilebackup2 — có thể mất vài phút)"
              onClick={async () => {
                const dir = await pickDirectory("Chọn thư mục lưu backup");
                if (!dir) return;
                try {
                  await backupDevice(device.udid, dir);
                  window.alert("Backup xong");
                } catch (e) {
                  window.alert(String(e));
                }
              }}
            >
              💾
            </button>
            <button
              type="button"
              title="Restore từ backup (GHI ĐÈ dữ liệu + khởi động lại)"
              onClick={async () => {
                const dir = await pickDirectory("Chọn thư mục backup để phục hồi");
                if (!dir) return;
                if (
                  !window.confirm(
                    "Phục hồi sẽ ghi đè dữ liệu trên thiết bị và khởi động lại. Tiếp tục?",
                  )
                ) {
                  return;
                }
                try {
                  await restoreDevice(device.udid, dir);
                  window.alert("Đã phục hồi (thiết bị sẽ khởi động lại)");
                } catch (e) {
                  window.alert(String(e));
                }
              }}
            >
              ⭯
            </button>
          </nav>

          <div className="focus-screen-wrap">
            <div className="focus-screen screen-only">
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

          <aside className="focus-toolbar">
            <h4>Điều khiển</h4>
            <button
              type="button"
              onClick={async () => {
                try {
                  await deviceHome(device.udid);
                } catch (e) {
                  window.alert(String(e));
                }
              }}
            >
              Home
            </button>
            <button
              type="button"
              onClick={async () => {
                try {
                  alert(`Đã lưu: ${await screenshot(device.udid)}`);
                } catch (e) {
                  window.alert(String(e));
                }
              }}
            >
              Chụp màn hình
            </button>
            <button
              type="button"
              onClick={async () => {
                try {
                  await rebootDevice(device.udid);
                } catch (e) {
                  window.alert(String(e));
                }
              }}
            >
              Khởi động lại
            </button>

            <h4>Bàn phím</h4>
            <label>
              Gõ chữ
              <input value={text} onChange={(e) => setText(e.target.value)} />
            </label>
            <button
              type="button"
              className="primary"
              onClick={async () => {
                try {
                  if (targets.length > 1) {
                    await groupInput({ udids: targets, kind: "type", text });
                  } else {
                    await deviceTypeText(device.udid, text);
                  }
                } catch (e) {
                  window.alert(String(e));
                }
              }}
            >
              Gửi phím
            </button>

            {groupMode && <p className="hint">Đồng bộ nhóm: {targets.length} máy</p>}
            <p className="hint">Click / kéo trên màn hình để điều khiển</p>
            <p className="hint mono">{device.udid}</p>
          </aside>
        </div>
      </div>
    </div>
  );
}
