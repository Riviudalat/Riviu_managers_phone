import { useCallback, useEffect, useState } from "react";

import { deviceHealth } from "../api";
import { describeError } from "../describeError";
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

  const mark = (ok: boolean | null | undefined, yes: string, no: string, unknown: string) => {
    if (ok === true) return `✓ ${yes}`;
    if (ok === false) return `✗ ${no}`;
    return `? ${unknown}`;
  };

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
          <p className="hint" role="alert">
            {error}
          </p>
        )}
        {!error && report === null && <p className="hint">Đang hỏi máy…</p>}
        {!error && report !== null && (
          <ul className="health-rows">
            <li>
              <strong>Roster:</strong>{" "}
              {report.rosterStatus ? report.rosterStatus : "không có trong danh sách"}
            </li>
            <li>
              <strong>Agent (cache):</strong> {report.agent.state}
              {report.agent.message ? ` — ${report.agent.message}` : ""}
            </li>
            <li>
              <strong>Agent (hỏi ngay):</strong>{" "}
              {mark(
                report.agentReadyNow,
                "đang trả lời /status",
                "không trả lời — nút Agent trên thanh công cụ là chỗ sửa",
                "backend không phải Android",
              )}
            </li>
            <li>
              <strong>Riviu helper:</strong>{" "}
              {mark(
                report.helperReachable,
                "đang chạy",
                report.helperInstalled === true
                  ? "đã cài nhưng chưa với tới được"
                  : report.helperInstalled === false
                    ? "chưa cài"
                    : "chưa với tới được (không hỏi được là đã cài hay chưa)",
                "backend không phải Android",
              )}
            </li>
            <li>
              <strong>Root:</strong>{" "}
              {report.root
                ? report.root.hasSu
                  ? "✓ có su (Magisk)"
                  : report.root.shellIsRoot
                    ? "✓ adb shell là root (không có su)"
                    : "✗ không root"
                : "? backend không phải Android"}
            </li>
            <li>
              <strong>TikTok:</strong>{" "}
              {report.tiktokPackage
                ? `${report.tiktokPackage} ${report.tiktokVersion ?? "(không đọc được version)"} · ${report.tiktokLocale ?? "(không đọc được locale)"}`
                : "không đọc được build"}
            </li>
            {report.notes.map((note) => (
              <li key={note} className="hint">
                {note}
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
