import { X } from "lucide-react";
import { useState } from "react";
import type { DeviceInfo, FlowTargetSelection } from "../../types";

type RunMode = FlowTargetSelection["mode"];

function targetSelection(
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
  const [chosenUdid, setChosenUdid] = useState(devices[0]?.udid ?? "");
  // `oneUdid` was initialised once from `devices[0]` and never reconciled. When that phone dropped
  // off the list the `<select>` fell back to displaying the first remaining option while the state
  // still held the departed one -- so Run submitted a device the operator could not see chosen, and
  // the backend refused or terminalised it instead of running the visible one.
  const oneUdid = devices.some((device) => device.udid === chosenUdid)
    ? chosenUdid
    : devices[0]?.udid ?? "";
  const selection = targetSelection(mode, oneUdid, selectedUdids);

  return (
    <section
      role="dialog"
      aria-modal="true"
      aria-label="Chạy Flow"
      className="flow-dialog flow-run-dialog"
    >
      <header>
        <strong>Chạy Flow</strong>
        {onClose && (
          <button type="button" aria-label="Đóng hộp thoại chạy" title="Đóng" onClick={onClose}>
            <X size={16} />
          </button>
        )}
      </header>
      <div className="segmented" role="radiogroup" aria-label="Thiết bị đích">
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
              {value === "one" ? "Một máy" : value === "selected" ? "Đã chọn" : "Tất cả máy hợp lệ"}
            </span>
          </label>
        ))}
      </div>
      {mode === "one" && (
        <select
          aria-label="Thiết bị"
          value={oneUdid}
          onChange={(event) => setChosenUdid(event.target.value)}
        >
          {devices.map((device) => (
            <option key={device.udid} value={device.udid}>
              {device.name}
            </option>
          ))}
        </select>
      )}
      {mode === "selected" && <output>{selectedUdids.length} máy đã chọn</output>}
      {mode === "allEligible" && <output>Đang tiền kiểm</output>}
      <footer>
        <button
          type="button"
          className="primary"
          disabled={selection === null}
          onClick={() => selection && onRun(selection)}
        >
          Chạy trên thiết bị
        </button>
      </footer>
    </section>
  );
}
