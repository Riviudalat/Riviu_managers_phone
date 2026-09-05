import { useState } from "react";
import { RefreshCw, Square } from "lucide-react";
import { operationCancelBatch } from "../api";
import { describeError } from "../describeError";
import type { OperationRunState } from "../types";
import type { useLibraryBatch } from "../useLibraryBatch";
import { LoadingState, StatusNotice } from "./States";
import { FormSection, ResponsiveTable, StatusChip } from "./WorkspacePrimitives";

const LABEL: Record<OperationRunState, string> = {
  queued: "Đang chờ", running: "Đang thực hiện", succeeded: "Đã xác nhận",
  failed: "Thất bại", uncertain: "Cần kiểm lại", cancelled: "Đã dừng trước khi chạy",
  skipped: "Đã bỏ qua", partial: "Một phần",
};

export function LibraryBatchMonitor({ batch, onRetry, retryDisabled = false }: {
  batch: ReturnType<typeof useLibraryBatch>;
  onRetry?: (artifactId: string, udids: string[]) => void;
  retryDisabled?: boolean;
}) {
  const [cancelling, setCancelling] = useState(false);
  const [cancelError, setCancelError] = useState<string | null>(null);
  const { detail } = batch;
  const retryUdids = detail?.items.filter((item) => item.retryable && item.udid).map((item) => item.udid!) ?? [];
  return (
    <FormSection title="Tiến độ theo máy">
      {batch.loading && <LoadingState label="Đang đọc các lần chạy đã lưu…" />}
      {batch.error && <StatusNotice tone="error" action={<button type="button" className="ghost" onClick={() => void batch.reload()}>Thử lại tiến độ</button>}>{batch.error}</StatusNotice>}
      {cancelError && <StatusNotice tone="error">{cancelError}</StatusNotice>}
      {detail && <>
        <div className="admin-toolbar">
          <strong>{detail.summary.title} · {detail.summary.completedItems}/{detail.summary.totalItems} máy</strong>
          <div className="admin-actions">
            {onRetry && detail.batch && retryUdids.length > 0 && <button type="button" className="ghost"
              disabled={retryDisabled || batch.active || !!batch.error}
              onClick={() => onRetry(detail.batch!.artifactId,retryUdids)}>
              <RefreshCw size={15} /> Chạy lại {retryUdids.length} máy
            </button>}
            <button type="button" className="ghost" onClick={() => void batch.reload()}><RefreshCw size={15} /> Đọc lại</button>
            {detail.items.some((item) => item.state === "queued") && <button type="button" className="ghost" disabled={cancelling} onClick={async () => {
              setCancelling(true); setCancelError(null);
              try { await operationCancelBatch(detail.summary.id); await batch.reload(); }
              catch (cause) { setCancelError(describeError(cause)); }
              finally { setCancelling(false); }
            }}><Square size={15} /> Dừng máy đang chờ</button>}
          </div>
        </div>
        <ResponsiveTable label="Tiến độ batch đã lưu" rows={detail.items} keyForRow={(item) => item.id} columns={[
          { id: "device", label: "Thiết bị", render: (item) => item.label },
          { id: "state", label: "Kết quả", render: (item) => <StatusChip tone={item.state === "succeeded" ? "success" : item.state === "failed" ? "error" : item.state === "uncertain" ? "warning" : "neutral"}>{LABEL[item.state]}</StatusChip> },
          { id: "detail", label: "Bằng chứng", render: (item) => <details><summary>Chi tiết</summary>{item.detail && <p>{item.detail}</p>}{item.evidence && <pre className="admin-raw">{item.evidence}</pre>}<code>{item.udid}</code>{item.errorCode && <code>{item.errorCode}</code>}</details> },
        ]} />
      </>}
      {!batch.loading && !batch.error && !detail && <p className="hint">Chưa có lần chạy nào.</p>}
    </FormSection>
  );
}
