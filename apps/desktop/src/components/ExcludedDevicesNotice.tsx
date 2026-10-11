import type { ExcludedDevice } from "../selectionEligibility";

interface Props {
  /** Phones that left a chosen set by themselves, in the order to list them. */
  excluded: readonly ExcludedDevice[];
  /** How the operator names a phone, normally "Máy N". */
  label: (udid: string) => string;
  onDismiss?: () => void;
  className?: string;
}

/**
 * The "Bị loại" line (PLAN §3): each phone that left the receiving set, and why.
 *
 * It says the two things the operator must not have to guess: these phones get no command,
 * and they do not come back by themselves when they free up. Only an explicit choice does.
 */
export function ExcludedDevicesNotice({ excluded, label, onDismiss, className }: Props) {
  if (!excluded.length) return null;
  return (
    <div className={["excluded-devices", className].filter(Boolean).join(" ")} role="status">
      <span>
        <strong>Bị loại ({excluded.length}):</strong>{" "}
        {excluded.map((entry) => `${label(entry.udid)} – ${entry.reason}`).join("; ")}
        . Không nhận lệnh; không tự chọn lại khi rảnh.
      </span>
      {onDismiss && (
        <button type="button" className="tb-btn" onClick={onDismiss}>Xóa danh sách bị loại</button>
      )}
    </div>
  );
}
