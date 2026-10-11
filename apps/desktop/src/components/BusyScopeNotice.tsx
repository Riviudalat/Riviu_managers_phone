import type { ExcludedDevice } from "../selectionEligibility";
import { ExcludedDevicesNotice } from "./ExcludedDevicesNotice";
import "../styles/busy-scope.css";

/** The scope line and "Bị loại" list for an automation run; the rules live in `useBusyScope`. */
export function BusyScopeNotice({ targets, runTargets, excluded, label, onTake, disabled, onlyWhenExcluded }: {
  targets: readonly string[];
  runTargets: readonly string[];
  excluded: readonly ExcludedDevice[];
  label: (udid: string) => string;
  onTake: (udid: string, label: string) => void;
  disabled?: boolean;
  /** Render nothing while no phone is excluded, for a page that already counts its phones. */
  onlyWhenExcluded?: boolean;
}) {
  if (onlyWhenExcluded && !excluded.length) return null;
  return <div className="busy-scope" aria-label="Phạm vi thực hiện">
    <p className="busy-scope-line" role="status">
      {targets.length} trong phạm vi · {runTargets.length} thực hiện · {excluded.length} bị loại
      {excluded.length > 0 && ` (${excluded.map(entry => `${label(entry.udid)} – ${entry.reason}`).join(", ")})`}
    </p>
    {excluded.length > 0 && <>
      <ExcludedDevicesNotice excluded={excluded} label={label}/>
      <div className="busy-scope-actions">{excluded.map(entry => <button key={entry.udid} type="button" className="ghost" disabled={disabled}
        aria-label={`Dừng việc cũ và dùng máy này · ${label(entry.udid)}`} onClick={() => onTake(entry.udid, label(entry.udid))}>
        Dừng việc cũ và dùng {label(entry.udid)}
      </button>)}</div>
    </>}
  </div>;
}
