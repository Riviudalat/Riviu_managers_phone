import { useEffect, useState } from "react";
import { RefreshCw } from "lucide-react";
import { operationGetRun } from "../api";
import { describeError } from "../describeError";
import type { OperationSourceRef } from "../operationSource";
import type { OperationRunDetail, OperationRunState } from "../types";
import { LoadingState, StatusNotice } from "./States";
import { ResponsiveTable, StatusChip } from "./WorkspacePrimitives";

const LABEL: Record<OperationRunState, string> = {
  queued: "Đang chờ", running: "Đang chạy", succeeded: "Hoàn tất", partial: "Một phần",
  failed: "Thất bại", uncertain: "Cần kiểm lại", cancelled: "Đã dừng", skipped: "Đã bỏ qua",
};

/** Reads a durable run, never substitutes the current live session for missing history. */
export function OperationSourceDetail({ source }: { source: OperationSourceRef }) {
  const [detail, setDetail] = useState<OperationRunDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    let active = true;
    setDetail(null); setError(null); setLoading(true);
    void operationGetRun(source.operationId).then((next) => {
      if (!active) return;
      if (!next || next.summary.id !== source.operationId || next.summary.kind !== source.kind || next.summary.sourceId !== source.sourceId) {
        throw new Error("Tác vụ được chọn không còn trong nguồn dữ liệu.");
      }
      if (source.itemId && !next.items.some((item) => item.id === source.itemId)) {
        throw new Error("Mục được chọn không còn trong tác vụ này.");
      }
      setDetail(next);
    }).catch((cause) => { if (active) setError(describeError(cause)); })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [retry, source.operationId, source.sourceId, source.kind, source.itemId]);
  return <section className="operation-source-detail" aria-label="Tác vụ từ lịch sử">
    {loading && <LoadingState label="Đang mở tác vụ được chọn…" />}
    {error && <StatusNotice tone="error" action={<button type="button" onClick={() => setRetry((value) => value + 1)}><RefreshCw size={16} /> Đọc lại tác vụ</button>}>{error}</StatusNotice>}
    {detail && <>
      <header className="admin-toolbar"><strong>{detail.summary.title}</strong><StatusChip>{LABEL[detail.summary.state]}</StatusChip></header>
      <ResponsiveTable label="Các mục của tác vụ được chọn" rows={source.itemId ? detail.items.filter((item) => item.id === source.itemId) : detail.items}
        keyForRow={(item) => item.id} columns={[
          { id: "name", label: "Mục", render: (item) => item.label },
          { id: "state", label: "Trạng thái", render: (item) => LABEL[item.state] },
          { id: "evidence", label: "Bằng chứng", render: (item) => <details><summary>Chi tiết</summary>{item.detail && <p>{item.detail}</p>}{item.errorCode && <code>{item.errorCode}</code>}{item.evidence && <pre className="admin-raw">{item.evidence}</pre>}</details> },
        ]} />
    </>}
  </section>;
}
