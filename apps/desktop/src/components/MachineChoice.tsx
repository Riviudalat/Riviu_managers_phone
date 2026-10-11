import { machineNumberLabel } from "../deviceNaming";
import type { ReactNode } from "react";
import { Smartphone } from "lucide-react";
import { deviceStatusText } from "../deviceWork";
import type { DevicePlatform } from "../types";
import { DeviceStatusLine } from "./DeviceStatusLine";
import { machineStatusView } from "./machineChoiceState";
import "../styles/machine-choice.css";

/** Shared two-column phone presentation; each workspace owns its selection rules. */
export function MachineChoice({ number, name, status, platform, checked, disabled, label, title, reason, onChange, detail }: {
  number: number;
  name: string;
  status: string;
  platform?: DevicePlatform;
  checked: boolean;
  disabled?: boolean;
  label: string;
  title?: string;
  reason?: string | null;
  onChange: (checked: boolean) => void;
  detail?: ReactNode;
}) {
  const ready = status === "ready";
  const view = machineStatusView(status, reason, platform);
  const state = deviceStatusText(view);
  return <article className={`machine-choice${checked ? " is-selected" : ""}${!ready ? " is-unavailable" : ""}`}>
    <label className="machine-choice-pick" title={title ?? `${machineNumberLabel(number)} · ${name} · ${state}`}>
      <Smartphone aria-hidden="true" />
      <span className="machine-choice-name"><strong>{machineNumberLabel(number)}</strong><span>{name}</span></span>
      <input type="checkbox" aria-label={label} checked={checked} disabled={disabled} onChange={(event) => onChange(event.target.checked)} />
    </label>
    <div className="machine-choice-status"><DeviceStatusLine view={view} /></div>
    {detail && <div className="machine-choice-detail">{detail}</div>}
    {!ready && reason && <p className="machine-choice-reason" title={reason}>{reason}</p>}
  </article>;
}
