import { useEffect, useState } from "react";
import type { PublishPreflightProgress } from "../../types";
import { publishStageLabel } from "../../features/operations/publishStartBridge";
import { ProgressBar } from "../ProgressBar";

function checksFraction(row?: PublishPreflightProgress): number | null {
  if (!row || row.stage === "preparingDevices" || !Number.isFinite(row.totalChecks) || row.totalChecks <= 0
    || !Number.isFinite(row.completedChecks) || row.completedChecks < 0) return null;
  return Math.min(row.state === "passed" ? 1 : .99, row.completedChecks / row.totalChecks);
}

export function PublishPreparationProgress({ udids, progress, startedAt, name }: {
  udids: string[];
  progress: Record<string, PublishPreflightProgress>;
  startedAt: number;
  name: (udid: string) => string;
}) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => { const timer = window.setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(timer); }, []);
  const shared = progress[""];
  const sharedRunning = shared?.state === "running" || shared?.state === "queued";
  const sharedLabel = shared && ({ checkingSheet: "Kiểm tra kết nối Sheet", staging: "Chuẩn bị nội dung",
    scanningSource: "Đang quét nội dung" } as Record<string, string>)[shared.stage];
  const finished = udids.filter(id => ["passed", "failed"].includes(progress[id]?.state)).length;
  const passed = udids.filter(id => progress[id]?.state === "passed").length;
  const rows = udids.map(id => progress[id]);
  const totalChecks = rows.reduce((sum, row) => sum + (row?.totalChecks ?? 0), 0);
  const completedChecks = rows.reduce((sum, row) => sum + Math.min(row?.completedChecks ?? 0, row?.totalChecks ?? 0), 0);
  const allPassed = udids.length > 0 && passed === udids.length;
  const failed = rows.some(row => row?.state === "failed");
  const totalFraction = !sharedRunning && rows.length > 0 && rows.every(row => checksFraction(row) !== null) && totalChecks > 0
    ? Math.min(allPassed ? 1 : .99, completedChecks / totalChecks) : null;
  return <section className="pw-preflight-pending" aria-label="Chuẩn bị từng máy">
    {sharedLabel && <div className="pw-preflight-phase" role="status">
      <div><strong>{sharedLabel}</strong><span>{sharedRunning ? "Đang xử lý" : shared.state === "failed" ? "Cần xử lý" : "Đã kiểm tra"}
        {` · ${(shared.elapsedMs / 1000).toFixed(1)} giây`}</span>{shared.error && <p role="alert">{shared.error}</p>}</div>
    </div>}
    <p role="status">Đã kiểm tra {finished}/{udids.length} máy · {passed} đạt · Tổng {Math.max(0, Math.floor((now - startedAt) / 1000))} giây</p>
    <ProgressBar label="Tiến độ kiểm tra toàn bộ máy" fraction={totalFraction} tone={failed ? "failed" : allPassed ? "done" : "run"} />
    <small>{sharedRunning ? "Kiểm tra điều kiện chung trước khi kiểm tra máy" : totalFraction === null ? "Đang chuẩn bị · chưa đủ số đo tổng" : `${completedChecks}/${totalChecks} bước kiểm tra đã xử lý`}</small>
    <div className="pw-preflight-pending-list" role="region" aria-label="Tiến độ từng máy" tabIndex={0}>
      {udids.map(udid => {
        const row = progress[udid];
        return <div key={udid}>
          <strong>{name(udid)}</strong>
          <span>{row ? ({ queued: "Đang chờ", running: "Đang kiểm tra", passed: "Đạt", failed: "Cần xử lý" }[row.state]) : "Chờ máy trả tiến độ"}</span>
          <small>{row ? `${row.completedChecks}/${row.totalChecks} bước · ${(row.elapsedMs / 1000).toFixed(1)} giây` : "Chưa có số đo"}</small>
          <div style={{ gridColumn: "1 / -1", width: "100%" }}>
            <ProgressBar label={`Tiến độ kiểm tra ${name(udid)}`} fraction={checksFraction(row)} tone={row?.state === "failed" ? "failed" : row?.state === "passed" ? "done" : "run"} />
            {row && <small>{publishStageLabel(row.stage)}</small>}
            {row?.error && <p role="alert">{row.error}</p>}
          </div>
        </div>;
      })}
    </div>
  </section>;
}
