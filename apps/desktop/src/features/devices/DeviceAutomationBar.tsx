import { Heart, MessageCircle, PanelRightClose, PanelRightOpen, Send, X } from "lucide-react";
import type { KeyboardEvent } from "react";

import type { DeviceAutomation } from "./deviceAutomation";

const DEVICE_AUTOMATIONS = [
  { id: "nurture", label: "Nuôi TikTok", Icon: Heart },
  { id: "interaction", label: "Tương tác", Icon: MessageCircle },
  { id: "publish", label: "Đăng bài", Icon: Send },
] as const;

export function DeviceAutomationBar({ value, onChange }: {
  value: DeviceAutomation | null;
  onChange: (value: DeviceAutomation | null) => Promise<boolean>;
}) {
  const select = async (next: DeviceAutomation, focus = false) => {
    if (await onChange(next) && focus) document.getElementById(`device-automation-${next}`)?.focus();
  };
  const close = async () => {
    if (value && await onChange(null)) document.getElementById(`device-automation-${value}`)?.focus();
  };
  const keyDown = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const next = event.key === "Home" ? 0 : event.key === "End" ? 2
      : (index + (event.key === "ArrowRight" ? 1 : 2)) % 3;
    void select(DEVICE_AUTOMATIONS[next].id, true);
  };
  return (
    <div className="device-automation-bar" onKeyDown={(event) => {
      if (event.key === "Escape" && value) { event.preventDefault(); void close(); }
    }}>
      <div role="tablist" aria-label="Tác vụ bên cạnh thiết bị">
        {DEVICE_AUTOMATIONS.map(({ id, label, Icon }, index) => (
          <button key={id} id={`device-automation-${id}`} type="button" role="tab"
            aria-selected={value === id} aria-controls="device-automation-panel"
            tabIndex={value === id || (value === null && index === 0) ? 0 : -1}
            onClick={() => void select(id)} onKeyDown={(event) => keyDown(event, index)}>
            <Icon size={16} aria-hidden="true" />{label}
          </button>
        ))}
      </div>
      {value && <button type="button" className="icon-button" title="Đóng khung tác vụ"
        aria-label="Đóng khung tác vụ" onClick={() => void close()}><X size={16} /></button>}
    </div>
  );
}

export function DeviceAutomationLayoutButton({ docked, onClick }: { docked: boolean; onClick: () => void }) {
  const Icon = docked ? PanelRightClose : PanelRightOpen;
  const label = docked ? "Mở trang tác vụ" : "Xem cùng thiết bị";
  return <button type="button" className="icon-button" title={label} aria-label={label} onClick={onClick}>
    <Icon size={18} aria-hidden="true" />
  </button>;
}
