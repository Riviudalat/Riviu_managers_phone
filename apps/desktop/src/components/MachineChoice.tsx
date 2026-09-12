import type { ReactNode } from "react";
import { Smartphone } from "lucide-react";
import { machineStatusLabel } from "./machineChoiceState";
import "../styles/machine-choice.css";

/** Shared two-column phone presentation; each workspace owns its selection rules. */
export function MachineChoice({ number, name, status, checked, disabled, label, title, reason, onChange, detail }: {
  number: number;
  name: string;
  status: string;
  checked: boolean;
  disabled?: boolean;
  label: string;
  title?: string;
  reason?: string | null;
  onChange: (checked: boolean) => void;
  detail?: ReactNode;
}) {
  const ready = status === "ready";
  const state = machineStatusLabel(status);
  return <article className={`machine-choice${checked ? " is-selected" : ""}${!ready ? " is-unavailable" : ""}`}>
    <label className="machine-choice-pick" title={title ?? `Máy ${number} · ${name} · ${state}`}>
      <Smartphone aria-hidden="true" />
      <span className="machine-choice-name"><strong>Máy {number}</strong><span>{name}</span></span>
      <input type="checkbox" aria-label={label} checked={checked} disabled={disabled} onChange={(event) => onChange(event.target.checked)} />
    </label>
    <div className={`machine-choice-status${ready ? " is-ready" : status === "error" ? " is-error" : ""}`}><span className="machine-choice-state" aria-hidden="true" />{state}</div>
    {detail && <div className="machine-choice-detail">{detail}</div>}
    {!ready && reason && <p className="machine-choice-reason" title={reason}>{reason}</p>}
  </article>;
}
