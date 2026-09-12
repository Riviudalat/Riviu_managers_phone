import { useModalFocus } from "./useModalFocus";
import { useCallback, useEffect, useState } from "react";
import { RefreshCw, X } from "lucide-react";

import { deviceHealth } from "../api";
import { describeError } from "../describeError";
import { normalizeDeviceHealth, type HealthStatus } from "../diagnostics";
import type { DeviceHealthReport, DeviceInfo } from "../types";
import { LoadingState, StatusNotice } from "./States";
import { StatusChip, type StatusTone } from "./WorkspacePrimitives";

const HEALTH_STATUS: Record<HealthStatus, { label: string; tone: StatusTone }> = {
  pass: { label: "Đạt", tone: "success" },
  warning: { label: "Cần xem", tone: "warning" },
  fail: { label: "Lỗi", tone: "error" },
  unknown: { label: "Chưa rõ", tone: "neutral" },
  notApplicable: { label: "Không áp dụng", tone: "neutral" },
};

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
  const dialogRef = useModalFocus<HTMLDivElement>(onClose);
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
        ref={dialogRef}
        tabIndex={-1}
        className="modal device-health"
        role="dialog"
        aria-modal="true"
        aria-label={`Kiểm tra ${device.name}`}
        onClick={(event) => event.stopPropagation()}
      >
        <header>
          <div className="device-modal-heading"><p>Chẩn đoán thiết bị</p><h2>Kiểm tra {device.name}</h2></div>
          <span className="grow" />
          <button type="button" className="ghost" onClick={load} disabled={busy}>
            <RefreshCw size={15} aria-hidden="true" />
            {busy ? "Đang kiểm…" : "Kiểm lại"}
          </button>
          <button type="button" className="icon-btn" onClick={onClose} aria-label="Đóng" title="Đóng">
            <X size={18} />
          </button>
        </header>

        <p className="hint">
          Trạng thái kết nối, điều khiển và luồng hình của máy. Mỗi mục hiển thị kết quả cùng bằng chứng của lần kiểm tra gần nhất.
        </p>

        {error && (
          <>
            <StatusNotice tone="error">Không đọc được trạng thái máy. Hãy kiểm lại.</StatusNotice>
            <details aria-label="Chi tiết lỗi kiểm tra máy">
              <summary>Chi tiết lỗi</summary>
              <pre>{error}</pre>
            </details>
          </>
        )}
        {!error && report === null && <LoadingState label="Đang hỏi máy…" />}
        {!error && report !== null && (
            <ul className="health-rows" aria-label="Chi tiết kiểm tra" aria-busy={busy}>
              {normalizeDeviceHealth(device, report).map((check) => (
                <li key={check.id} data-health-status={check.status}>
                  <div><strong>{check.label}</strong><StatusChip tone={HEALTH_STATUS[check.status].tone}>{HEALTH_STATUS[check.status].label}</StatusChip></div>
                  <p>{check.summary}</p>
                  {check.detail && <details><summary>Bằng chứng kỹ thuật</summary><pre>{check.detail}</pre></details>}
                </li>
              ))}
            </ul>
        )}
      </div>
    </div>
  );
}
