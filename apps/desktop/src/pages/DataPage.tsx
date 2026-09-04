import { useEffect, useRef, useState } from "react";
import { RefreshCw } from "lucide-react";

import { analyticsSummary } from "../api";
import { OperationLog } from "../components/OperationLog";
import { FormSection, StatusChip } from "../components/WorkspacePrimitives";
import { LoadingState, StatusNotice } from "../components/States";
import type { AnalyticsSummary } from "../types";
import { describeError } from "../describeError";

/** Fleet analytics projected only from durable application data. */
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
    <div className="admin-workspace data-workspace">
      {loading && !data && <LoadingState label="Đang tải dữ liệu…" />}
      {err && !data && (
        <StatusNotice
          tone="error"
          action={<button type="button" className="ghost" onClick={() => void load()}>Thử lại</button>}
        >
          Không tải được dữ liệu: {err}
        </StatusNotice>
      )}
      {data && (
        <main className="admin-main">
          <div className="admin-toolbar">
            <div className="admin-toolbar-copy">
              <strong>Tổng quan vận hành</strong>
              <span>Số liệu được tổng hợp từ trạng thái và lịch sử hiện có.</span>
            </div>
            <div className="admin-toolbar-actions">
              <StatusChip tone={data.jobsFailed ? "warning" : "success"}>
                {data.jobsFailed ? `${data.jobsFailed} tác vụ lỗi` : "Không có tác vụ lỗi"}
              </StatusChip>
              <button type="button" className="ghost" onClick={() => void load()} disabled={loading}>
                <RefreshCw size={15} aria-hidden="true" />
                {loading ? "Đang làm mới…" : "Làm mới"}
              </button>
            </div>
          </div>
          {err && (
            <StatusNotice
              tone="error"
              action={<button type="button" className="ghost" onClick={() => void load()}>Thử lại</button>}
            >
              Không làm mới được dữ liệu: {err}
            </StatusNotice>
          )}

          <FormSection title="Năng lực hiện tại">
            <dl className="admin-metric-grid">
              <div className="admin-metric"><dt>Thiết bị</dt><dd>{data.deviceReady}/{data.deviceTotal}</dd></div>
              <div className="admin-metric"><dt>Đang chạy</dt><dd>{data.jobsRunning}</dd></div>
              <div className="admin-metric"><dt>Đã thành công</dt><dd>{data.jobsSucceeded}</dd></div>
              <div className="admin-metric"><dt>Thất bại</dt><dd>{data.jobsFailed}</dd></div>
              <div className="admin-metric"><dt>Flow</dt><dd>{data.scriptsTotal}</dd></div>
              <div className="admin-metric"><dt>Nội dung</dt><dd>{data.materialsTotal}</dd></div>
              <div className="admin-metric"><dt>Ứng dụng</dt><dd>{data.appsTotal}</dd></div>
              <div className="admin-metric"><dt>Lịch đang bật</dt><dd>{data.schedulesEnabled}</dd></div>
            </dl>
          </FormSection>

          <FormSection title="Nhật ký thao tác" description="Các ghi nhận gần nhất từ runtime đang chạy.">
            <OperationLog />
          </FormSection>
        </main>
      )}
    </div>
  );
}
