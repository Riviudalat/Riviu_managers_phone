import type { ExcludedDevice } from "../selectionEligibility";
import { ExcludedDevicesNotice } from "./ExcludedDevicesNotice";
import "../styles/busy-scope.css";

/** The scope line and "Bị loại" list for an automation run; the rules live in `useBusyScope`. */
export function BusyScopeNotice({ targets, runTargets, excluded, label, onlyWhenExcluded }: {
  targets: readonly string[];
  runTargets: readonly string[];
  excluded: readonly ExcludedDevice[];
  label: (udid: string) => string;
  /** Render nothing while no phone is excluded, for a page that already counts its phones. */
  onlyWhenExcluded?: boolean;
}) {
  if (onlyWhenExcluded && !excluded.length) return null;
  return <div className="busy-scope" aria-label="Phạm vi thực hiện">
    <p className="busy-scope-line" role="status">
      {targets.length} trong phạm vi · {runTargets.length} thực hiện · {excluded.length} bị loại
    </p>
    {/* The per-phone list and the way out open on demand: on a 560 px window three lines of
        repeated reasons left the Tương tác step 10 px (UIX, 820x560 mock). */}
    <ExcludedDevicesNotice excluded={excluded} label={label}
      hint="Chưa thể giành máy bận từ trang này. Dừng việc cũ trong Lượt chạy, rồi bỏ máy khỏi phạm vi và chọn lại."/>
  </div>;
}
