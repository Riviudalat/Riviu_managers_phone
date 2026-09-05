import { useCallback, useEffect, useRef, useState } from "react";
import { RefreshCw } from "lucide-react";
import { localApiStatus, type LocalApiStatus } from "../../api";
import { describeError } from "../../describeError";
import { LoadingState, StatusNotice } from "../States";
import { StatusChip } from "../WorkspacePrimitives";

export function ApiRuntimeStatus() {
  const [status, setStatus] = useState<LocalApiStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const ticket = useRef(0);
  const load = useCallback(async () => {
    const current = ++ticket.current;
    setLoading(true); setError(null);
    try {
      const next = await localApiStatus();
      if (ticket.current === current) setStatus(next);
    } catch (cause) { if (ticket.current === current) setError(describeError(cause)); }
    finally { if (ticket.current === current) setLoading(false); }
  }, []);
  useEffect(() => { void load(); return () => { ticket.current += 1; }; }, [load]);
  return <section className="api-runtime-status" aria-label="Kết nối Local API">
    {loading && !status && <LoadingState label="Đang đọc kết nối API…" />}
    {error ? <StatusNotice tone="error" action={<button type="button" className="ghost" onClick={() => void load()}>Kiểm lại kết nối API</button>}>
      Chưa xác định kết nối API: {error}
    </StatusNotice> : status && <>
      <StatusChip tone={status.lastError ? "error" : status.running === null ? "warning" : status.running ? "success" : "neutral"}>
        {status.lastError ? "Kết nối gặp lỗi" : status.running === null ? "Chưa xác định" : status.running ? "Đang lắng nghe" : "Đã tắt"}
      </StatusChip>
      {status.running && status.activePort !== null && <p><code>127.0.0.1:{status.activePort}</code></p>}
      {status.restartRequired && <StatusNotice tone="warning">Cấu hình mới có hiệu lực sau khi khởi động lại ứng dụng.</StatusNotice>}
      {status.lastError && <details className="admin-detail"><summary>Chi tiết lỗi</summary><p>{status.lastError}</p></details>}
    </>}
    <button type="button" className="icon-btn" title="Làm mới trạng thái API" aria-label="Làm mới trạng thái API" onClick={() => void load()} disabled={loading}>
      <RefreshCw size={16} aria-hidden="true" />
    </button>
  </section>;
}
