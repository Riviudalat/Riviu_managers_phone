import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { ScanLine, Smartphone, X } from "lucide-react";

import type { DeviceInfo } from "../types";
import { DeviceInspector } from "./DeviceInspector";
import { useModalFocus } from "./useModalFocus";
import { INSPECTOR_PICKER_REQUEST } from "./inspectorPicker";
import "./inspector-launcher.css";

function InspectorPicker({ devices, deviceLabels, onSelect, onClose }: {
  devices: DeviceInfo[];
  deviceLabels: Map<string, string>;
  onSelect: (udid: string) => void;
  onClose: () => void;
}) {
  const ref = useModalFocus<HTMLDivElement>(onClose);
  return createPortal(
    <div className="modal-backdrop">
      <div ref={ref} tabIndex={-1} className="modal inspector-picker" role="dialog" aria-modal="true" aria-label="Chọn máy Android cho Inspector">
        <header>
          <h2>Chọn máy Android</h2>
          <button type="button" className="icon-btn" aria-label="Đóng danh sách máy" onClick={onClose}><X size={18} /></button>
        </header>
        {devices.length ? (
          <div className="inspector-picker-list">
            {devices.map((device) => (
              <button key={device.udid} type="button" onClick={() => onSelect(device.udid)}>
                <Smartphone size={17} aria-hidden="true" />
                <span>{deviceLabels.get(device.udid) ?? device.name}</span>
              </button>
            ))}
          </div>
        ) : <p>Không có máy Android đang kết nối.</p>}
      </div>
    </div>,
    document.body,
  );
}

export function InspectorLauncher({ devices, deviceLabels }: {
  devices: DeviceInfo[];
  deviceLabels: Map<string, string>;
}) {
  const [pickerOpen, setPickerOpen] = useState(false);
  const [selectedUdid, setSelectedUdid] = useState<string | null>(null);
  const connectedAndroid = devices.filter((device) => device.platform === "android" && device.status !== "disconnected");
  const selectedConnected = connectedAndroid.some((device) => device.udid === selectedUdid);

  if (selectedUdid && !selectedConnected) setSelectedUdid(null);

  useEffect(() => {
    const open = () => setPickerOpen(true);
    window.addEventListener(INSPECTOR_PICKER_REQUEST, open);
    return () => window.removeEventListener(INSPECTOR_PICKER_REQUEST, open);
  }, []);

  return <>
    <button type="button" className="inspector-launcher" title="Bắt thuộc tính & ghi Flow (chỉ máy Android)" aria-label="Mở Inspector" onClick={() => setPickerOpen(true)}>
      <ScanLine size={16} aria-hidden="true" />
      <span>Inspector</span>
    </button>
    {pickerOpen && <InspectorPicker
      devices={connectedAndroid}
      deviceLabels={deviceLabels}
      onClose={() => setPickerOpen(false)}
      onSelect={(udid) => { setPickerOpen(false); setSelectedUdid(udid); }}
    />}
    {selectedUdid && selectedConnected && <DeviceInspector key={selectedUdid} udid={selectedUdid} onClose={() => setSelectedUdid(null)} />}
  </>;
}
