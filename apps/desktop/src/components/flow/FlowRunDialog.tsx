import { X } from "lucide-react";
import { useState } from "react";
import type { DeviceInfo, FlowTargetSelection } from "../../types";

type RunMode = FlowTargetSelection["mode"];

export function targetSelection(
  mode: RunMode,
  oneUdid: string,
  selectedUdids: string[],
): FlowTargetSelection | null {
  if (mode === "one") return oneUdid ? { mode: "one", udid: oneUdid } : null;
  if (mode === "selected") {
    const udids = [...new Set(selectedUdids)].sort();
    return udids.length > 0 ? { mode: "selected", udids } : null;
  }
  return { mode: "allEligible" };
}

export function FlowRunDialog({
  devices,
  selectedUdids,
  onRun,
  onClose,
}: {
  devices: DeviceInfo[];
  selectedUdids: string[];
  onRun: (selection: FlowTargetSelection) => void;
  onClose?: () => void;
}) {
  const [mode, setMode] = useState<RunMode>("selected");
  const [oneUdid, setOneUdid] = useState(devices[0]?.udid ?? "");
  const selection = targetSelection(mode, oneUdid, selectedUdids);

  return (
    <section role="dialog" aria-label="Run flow" className="flow-dialog flow-run-dialog">
      <header>
        <strong>Run flow</strong>
        {onClose && (
          <button type="button" aria-label="Close run dialog" title="Close" onClick={onClose}>
            <X size={16} />
          </button>
        )}
      </header>
      <div className="segmented" role="radiogroup" aria-label="Targets">
        {(["one", "selected", "allEligible"] as const).map((value) => (
          <label key={value}>
            <input
              type="radio"
              name="flow-target-mode"
              value={value}
              checked={mode === value}
              onChange={() => setMode(value)}
            />
            <span>
              {value === "one" ? "One" : value === "selected" ? "Selected" : "All eligible"}
            </span>
          </label>
        ))}
      </div>
      {mode === "one" && (
        <select
          aria-label="Device"
          value={oneUdid}
          onChange={(event) => setOneUdid(event.target.value)}
        >
          {devices.map((device) => (
            <option key={device.udid} value={device.udid}>
              {device.name}
            </option>
          ))}
        </select>
      )}
      {mode === "selected" && <output>{selectedUdids.length} selected</output>}
      {mode === "allEligible" && <output>Preflight pending</output>}
      <footer>
        <button
          type="button"
          className="primary"
          disabled={selection === null}
          onClick={() => selection && onRun(selection)}
        >
          Run on devices
        </button>
      </footer>
    </section>
  );
}
