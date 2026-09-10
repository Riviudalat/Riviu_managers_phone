import type { KeyboardEvent } from "react";
import "../styles/automation-tabs.css";

export type AutomationMode = "setup" | "schedule" | "monitor";
const modes = [["setup", "Thiết lập"], ["schedule", "Hẹn giờ"], ["monitor", "Theo dõi"]] as const;

export function AutomationTabs({ id, label, value, onChange }: {
  id: string; label: string; value: AutomationMode; onChange: (mode: AutomationMode) => void;
}) {
  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    const current = modes.findIndex(([mode]) => mode === value);
    const index = event.key === "Home" ? 0 : event.key === "End" ? modes.length - 1
      : event.key === "ArrowRight" ? (current + 1) % modes.length
        : event.key === "ArrowLeft" ? (current - 1 + modes.length) % modes.length : null;
    if (index === null) return;
    event.preventDefault();
    onChange(modes[index][0]);
    document.getElementById(`${id}-tab-${modes[index][0]}`)?.focus();
  };
  return <div className="automation-page-tabs" role="tablist" aria-label={label}>{modes.map(([mode, text]) =>
    <button key={mode} type="button" role="tab" id={`${id}-tab-${mode}`} aria-controls={`${id}-panel-${mode}`}
      aria-selected={mode === value} tabIndex={mode === value ? 0 : -1}
      onClick={() => onChange(mode)} onKeyDown={onKeyDown}>{text}</button>)}</div>;
}
