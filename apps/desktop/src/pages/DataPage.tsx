import { useEffect, useRef, useState } from "react";
import { analyticsSummary } from "../api";
import { OperationLog } from "../components/OperationLog";
import { LoadingState, StatusNotice } from "../components/States";
import type { AnalyticsSummary } from "../types";
import { describeError } from "../describeError";

/** Fleet analytics, read-only. */
export function DataPage() {
  const [data, setData] = useState<AnalyticsSummary | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const loadTicket = useRef(0);
  const load = async () => {
    const ticket = ++loadTicket.current;
    setLoading(true);
    setErr(null);
    try {
      const next = await analyticsSummary();
      if (ticket === loadTicket.current) setData(next);
    } catch (error) {
      if (ticket === loadTicket.current) setErr(describeError(error));
    } finally {
      if (ticket === loadTicket.current) setLoading(false);
    }
  };
  useEffect(() => {
    void load();
    return () => {
      loadTicket.current += 1;
    };
  }, []);

  return (
    <div className="panel">
      {loading && !data && <LoadingState label="Đang tải dữ liệu…" />}
      {err && !data && (
        <StatusNotice
          tone="error"
          action={(
            <button type="button" className="ghost" onClick={() => void load()}>
              Thử lại
            </button>
          )}
        >
          Không tải được dữ liệu: {err}
        </StatusNotice>
      )}
      {data && (
        <>
          <div className="panel-header" style={{ justifyContent: "flex-end" }}>
            <button type="button" className="ghost" onClick={() => void load()} disabled={loading}>
              Làm mới
            </button>
          </div>
          {err && (
            <StatusNotice
              tone="error"
              action={(
                <button type="button" className="ghost" onClick={() => void load()}>
                  Thử lại
                </button>
              )}
            >
              Không làm mới được dữ liệu: {err}
            </StatusNotice>
          )}
          <div className="stats-grid">
            {(
              [
                ["Thiết bị", `${data.deviceReady}/${data.deviceTotal}`],
                ["Tác vụ thành công", String(data.jobsSucceeded)],
                ["Tác vụ lỗi", String(data.jobsFailed)],
                ["Đang chạy", String(data.jobsRunning)],
                ["Kịch bản", String(data.scriptsTotal)],
                ["Nội dung", String(data.materialsTotal)],
                ["Ứng dụng", String(data.appsTotal)],
                ["Lịch đang bật", String(data.schedulesEnabled)],
              ] as const
            ).map(([k, v]) => (
              <article key={k} className="job-card">
                <div className="hint">{k}</div>
                <strong style={{ fontSize: "1.4rem" }}>{v}</strong>
              </article>
            ))}
          </div>
          <OperationLog />
        </>
      )}
    </div>
  );
}
