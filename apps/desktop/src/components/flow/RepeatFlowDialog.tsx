import { Repeat2, X } from "lucide-react";
import { useState } from "react";
import { analyzeFlowRepeat, repeatLinearFlow } from "../../repeatFlow";
import type { FlowDocumentV2 } from "../../types";
import { useModalFocus } from "../useModalFocus";

export function RepeatFlowDialog({ document, onApply, onClose }: { document: FlowDocumentV2; onApply: (document: FlowDocumentV2) => void; onClose: () => void }) {
  const [count, setCount] = useState("2");
  const [error, setError] = useState<string | null>(null);
  const dialogRef = useModalFocus<HTMLElement>(onClose);
  const value = count.trim() ? Number(count) : NaN;
  const analysis = analyzeFlowRepeat(document, value);
  return (
    <section ref={dialogRef} tabIndex={-1} role="dialog" aria-modal="true" aria-label="Lặp chuỗi hành động" className="flow-dialog">
      <header><strong>Lặp chuỗi hành động</strong><button type="button" aria-label="Đóng hộp thoại lặp" onClick={onClose}><X size={16} /></button></header>
      <p>Tạo nhiều lượt cho toàn bộ hành động giữa Bắt đầu và Kết thúc. Mỗi lượt sẽ có bước và kết quả thực hiện riêng.</p>
      <label className="flow-field"><span>Tổng số lượt</span><input type="number" min={1} max={50} step={1} value={count} onChange={(event) => { setCount(event.currentTarget.value); setError(null); }} /></label>
      {analysis.error ? <p role="alert">{analysis.error}</p> : <output aria-label="Kết quả lặp">{analysis.body.length} hành động × {value} lượt = {analysis.totalNodeCount} node, gồm Bắt đầu và Kết thúc.{analysis.waitDurationMs > 0 ? ` Tổng thời gian chờ đã cấu hình: ${analysis.waitDurationMs / 1000} giây; chưa tính thời gian thao tác.` : ""}</output>}
      <p>Bản nháp cần được kiểm tra và lưu trước khi chạy. Hoàn tác sẽ khôi phục chuỗi trước khi lặp.</p>
      {error && <p role="alert">{error}</p>}
      <footer><button type="button" onClick={onClose}>Hủy</button><button type="button" disabled={analysis.error !== null || value === 1} onClick={() => {
        try { onApply(repeatLinearFlow(document, value)); } catch (reason) { setError(reason instanceof Error ? reason.message : "Tạo chuỗi lặp thất bại."); }
      }}><Repeat2 size={15} aria-hidden="true" />Tạo các lượt</button></footer>
    </section>
  );
}
