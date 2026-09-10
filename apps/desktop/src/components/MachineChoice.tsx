import type { ReactNode } from "react";
import { Smartphone } from "lucide-react";
import "../styles/machine-choice.css";

/** Shared two-column phone presentation; each workspace owns its selection rules. */
export function MachineChoice({ number, name, status, checked, disabled, label, title, onChange, detail }: {
  number: number;
  name: string;
  status: string;
  checked: boolean;
  disabled?: boolean;
  label: string;
  title?: string;
  onChange: (checked: boolean) => void;
  detail?: ReactNode;
}) {
  const ready = status === "ready";
  const state = ready ? "Sẵn sàng" : status === "busy" ? "Đang bận" : "Chưa sẵn sàng";
  const indicator = <span className={`machine-choice-state${ready ? " is-ready" : ""}`} role="img" aria-label={state} title={state} />;
  return <article className={`machine-choice${checked ? " is-selected" : ""}${!ready ? " is-unavailable" : ""}`}>
    <label className="machine-choice-pick" title={title ?? `Máy ${number} · ${name} · ${state}`}>
      <Smartphone aria-hidden="true" />
      <span className="machine-choice-name"><strong>Máy {number}</strong><span>{name}</span></span>
      {!detail && indicator}
      <input type="checkbox" aria-label={label} checked={checked} disabled={disabled} onChange={(event) => onChange(event.target.checked)} />
    </label>
    {detail && <div className="machine-choice-detail">{indicator}{detail}</div>}
  </article>;
}
