import { useState } from "react";
import { createPortal } from "react-dom";
import { ScanLine, Smartphone, X } from "lucide-react";

import type { DeviceInfo } from "../types";
import { DeviceInspector } from "./DeviceInspector";
import { useModalFocus } from "./useModalFocus";
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

  return <>
    <button type="button" className="icon-btn" title="Bắt thuộc tính & ghi Flow" aria-label="Mở Inspector" onClick={() => setPickerOpen(true)}>
      <ScanLine size={17} aria-hidden="true" />
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
