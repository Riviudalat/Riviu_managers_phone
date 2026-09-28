import { useEffect, useState } from "react";
import type { PublishPreflightProgress } from "../../types";
import { publishStageLabel } from "../../features/operations/publishStartBridge";
import { PublishPager } from "./PublishPager";
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
  const [page, setPage] = useState(0);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => { const timer = window.setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(timer); }, []);
  const current = Math.min(page, Math.max(0, Math.ceil(udids.length / 8) - 1));
  const finished = udids.filter(id => ["passed", "failed"].includes(progress[id]?.state)).length;
  const passed = udids.filter(id => progress[id]?.state === "passed").length;
  const rows = udids.map(id => progress[id]);
  const totalChecks = rows.reduce((sum, row) => sum + (row?.totalChecks ?? 0), 0);
  const completedChecks = rows.reduce((sum, row) => sum + Math.min(row?.completedChecks ?? 0, row?.totalChecks ?? 0), 0);
  const allPassed = udids.length > 0 && passed === udids.length;
  const failed = rows.some(row => row?.state === "failed");
  const totalFraction = rows.length > 0 && rows.every(row => checksFraction(row) !== null) && totalChecks > 0
    ? Math.min(allPassed ? 1 : .99, completedChecks / totalChecks) : null;
  return <section className="pw-preflight-pending" aria-label="Chuẩn bị từng máy">
    <p role="status">Đã kiểm tra {finished}/{udids.length} máy · {passed} đạt · Tổng {Math.max(0, Math.floor((now - startedAt) / 1000))} giây</p>
    <ProgressBar label="Tiến độ kiểm tra toàn bộ máy" fraction={totalFraction} tone={failed ? "failed" : allPassed ? "done" : "run"} />
    <small>{totalFraction === null ? "Đang chuẩn bị · chưa đủ số đo tổng" : `${completedChecks}/${totalChecks} bước kiểm tra đã xử lý`}</small>
    <div className="pw-preflight-pending-list">
      {udids.slice(current * 8, (current + 1) * 8).map(udid => {
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
    <PublishPager label="Chuẩn bị máy" page={current} size={8} total={udids.length} onPage={setPage} />
  </section>;
}
