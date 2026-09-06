import { ScanSearch } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { interactionReadback, type InteractionReadback } from "../../api";
import { describeError } from "../../describeError";

export function InteractionReadbackControl({ campaignId, assignmentId, disabled }: { campaignId: string; assignmentId: string; disabled: boolean }) {
  const [reading, setReading] = useState<InteractionReadback | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const ticket = useRef(0);
  const invalidate = useCallback(() => { ticket.current++; }, []);
  useEffect(() => { invalidate(); setReading(null); setError(null); setBusy(false); return invalidate; }, [campaignId, assignmentId, invalidate]);
  async function check() {
    const request = ++ticket.current; setBusy(true); setError(null);
    try { const next = await interactionReadback(campaignId, assignmentId); if (ticket.current === request) setReading(next); }
    catch (e) { if (ticket.current === request) setError(describeError(e)); }
    finally { if (ticket.current === request) setBusy(false); }
  }
  return <div className="interaction-readback">
    <button type="button" className="btn btn-sm" disabled={disabled || busy} onClick={() => void check()}><ScanSearch size={14} aria-hidden="true" />{busy ? "Đang kiểm tra…" : "Kiểm tra lại kết quả"}</button>
    {error && <small role="alert">{error}</small>}
    {reading && <div role="status">
      <small>Hiện tại: Tim {({ present: "đang có", absent: "chưa có", unknown: "chưa rõ" }[reading.like]) ?? "chưa rõ"}; Lưu {({ saved: "đang có", unsaved: "chưa có", unreadable: "chưa rõ" }[reading.save]) ?? "chưa rõ"}.</small>
      <small>Bình luận chưa được đối chiếu lại. Lịch sử gửi giữ nguyên.</small>
      <time dateTime={reading.checkedAt}>{new Date(reading.checkedAt).toLocaleString("vi-VN")}</time>
      <details><summary>Bằng chứng kiểm tra</summary><a href={reading.targetUrl} target="_blank" rel="noreferrer">Bài đã đối chiếu</a><code>{reading.snapshotSha256}</code></details>
    </div>}
  </div>;
}
