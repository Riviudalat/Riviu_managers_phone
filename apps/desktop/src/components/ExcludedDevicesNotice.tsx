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
      {/* One short line; the per-phone list opens on demand so 20 phones never fill the bar. */}
      <details className="excluded-devices-list">
        <summary>
          <strong>Bị loại ({excluded.length})</strong>
          {" · "}{summarizeReasons(excluded)} · không nhận lệnh, không tự chọn lại khi rảnh
        </summary>
        <span>{excluded.map((entry) => `${label(entry.udid)} – ${entry.reason}`).join("; ")}.</span>
      </details>
      {onDismiss && (
        <button type="button" className="tb-btn" onClick={onDismiss}>Xóa danh sách bị loại</button>
      )}
    </div>
  );
}

/** "20 máy: Đang chạy: Tương tác" — the reasons grouped, most common first. */
function summarizeReasons(excluded: readonly ExcludedDevice[]): string {
  const counts = new Map<string, number>();
  for (const entry of excluded) counts.set(entry.reason, (counts.get(entry.reason) ?? 0) + 1);
  return [...counts.entries()]
    .sort((a, b) => b[1] - a[1])
    .slice(0, 2)
    .map(([reason, count]) => `${count} máy: ${reason}`)
    .join("; ") + (counts.size > 2 ? "; …" : "");
}
