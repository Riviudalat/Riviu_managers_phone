import { useEffect, useRef, useState } from "react";
import { RefreshCw } from "lucide-react";

import { analyticsSummary, operationQueryRuns } from "../api";
import { OperationLog } from "../components/OperationLog";
import { FormSection, StatusChip } from "../components/WorkspacePrimitives";
import { LoadingState, StatusNotice } from "../components/States";
import type { AnalyticsSummary, OperationRunPage } from "../types";
import { describeError } from "../describeError";

/** Fleet analytics projected only from durable application data. */
export function DataPage() {
  const [data, setData] = useState<AnalyticsSummary | null>(null);
  const [operations, setOperations] = useState<OperationRunPage | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const loadTicket = useRef(0);
  const load = async () => {
    const ticket = ++loadTicket.current;
    setLoading(true);
    setErr(null);
    try {
      const [next, runs] = await Promise.all([analyticsSummary(), operationQueryRuns({ since: new Date(Date.now() - 86400000).toISOString(), limit: 1 })]);
      if (ticket === loadTicket.current) { setData(next); setOperations(runs); }
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
              <span>Tác vụ trong 24 giờ qua</span>
            </div>
            <div className="admin-toolbar-actions">
              <StatusChip tone={operations?.counts.attention ? "warning" : "success"}>
                {operations?.counts.attention ? `${operations.counts.attention} tác vụ cần xử lý` : "Không có tác vụ cần xử lý trong 24 giờ"}
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
              <div className="admin-metric"><dt>Đang chạy</dt><dd>{operations?.counts.active ?? "—"}</dd></div>
              <div className="admin-metric"><dt>Đã thành công</dt><dd>{operations?.counts.succeeded ?? "—"}</dd></div>
              <div className="admin-metric"><dt>Cần xử lý</dt><dd>{operations?.counts.attention ?? "—"}</dd></div>
              <div className="admin-metric"><dt>Tổng tác vụ</dt><dd>{operations?.total ?? "—"}</dd></div>
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
