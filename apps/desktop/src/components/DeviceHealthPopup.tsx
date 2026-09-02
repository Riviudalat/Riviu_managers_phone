import { useCallback, useEffect, useState } from "react";

import { deviceHealth } from "../api";
import { describeError } from "../describeError";
import { normalizeDeviceHealth } from "../diagnostics";
import type { DeviceHealthReport, DeviceInfo } from "../types";

/**
 * "Kiểm tra máy": one phone's health, section by section, read-only.
 *
 * Every row here is the answer to a question some refusal elsewhere in the app is written
 * in — agent not ready, helper unreachable, build not measured — surfaced BEFORE the
 * refusal, on demand, without taking a lease or changing the phone. A section that could
 * not be asked renders as its own note rather than a blank: "chưa với tới được" is a
 * different diagnosis from "không có", and collapsing them is how phones get re-flashed
 * for a transport problem.
 */
export function DeviceHealthPopup({
  device,
  onClose,
}: {
  device: DeviceInfo;
  onClose: () => void;
}) {
  const [report, setReport] = useState<DeviceHealthReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(() => {
    setBusy(true);
    setError(null);
    void deviceHealth(device.udid)
      .then((next) => setReport(next))
      .catch((cause) => setError(describeError(cause)))
      .finally(() => setBusy(false));
  }, [device.udid]);

  useEffect(load, [load]);

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal device-health"
        role="dialog"
        aria-modal="true"
        aria-label={`Kiểm tra ${device.name}`}
        onClick={(event) => event.stopPropagation()}
      >
        <header className="panel-header">
          <h2>Kiểm tra {device.name}</h2>
          <span className="grow" />
          <button type="button" className="ghost" onClick={load} disabled={busy}>
            {busy ? "Đang kiểm…" : "Kiểm lại"}
          </button>
          <button type="button" className="ghost" onClick={onClose}>
            Đóng
          </button>
        </header>

        <p className="hint">
          Chỉ đọc — không giữ máy, không cài gì, không đổi gì. Mục nào không hỏi được sẽ nói
          rõ là không hỏi được, thay vì đoán.
        </p>

        {error && (
          <>
            <p className="hint" role="alert">Không đọc được trạng thái máy. Hãy kiểm lại.</p>
            <details aria-label="Chi tiết lỗi kiểm tra máy">
              <summary>Chi tiết lỗi</summary>
              <pre>{error}</pre>
            </details>
          </>
        )}
        {!error && report === null && <p className="hint">Đang hỏi máy…</p>}
        {!error && report !== null && (
          <details className="health-rows" open>
            <summary>Chi tiết kiểm tra</summary>
            <ul>
              {normalizeDeviceHealth(device, report).map((check) => (
                <li key={check.id} data-health-status={check.status}>
                  <strong>{check.label}:</strong> {check.summary}
                  {check.detail && <small>{check.detail}</small>}
                </li>
              ))}
            </ul>
          </details>
        )}
      </div>
    </div>
  );
}
