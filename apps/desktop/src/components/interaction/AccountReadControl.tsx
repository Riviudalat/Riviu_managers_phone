import { ScanSearch } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { interactionReadAccount, type AccountReading } from "../../api";
import { describeError } from "../../describeError";

export function AccountReadControl({ udid, handle, disabled }: { udid: string; handle: string; disabled: boolean }) {
  const [reading, setReading] = useState<AccountReading | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const ticket = useRef(0);
  const invalidate = useCallback(() => { ticket.current++; }, []);
  useEffect(() => { invalidate(); setReading(null); setError(null); setBusy(false); return invalidate; }, [udid, handle, invalidate]);
  async function check() {
    const request = ++ticket.current;
    setBusy(true); setError(null);
    try {
      const next = await interactionReadAccount(udid);
      if (ticket.current !== request) return;
      if (next.expectedHandle.trim().replace(/^@+/, "").toLowerCase() !== handle.trim().replace(/^@+/, "").toLowerCase()) throw new Error("Nick đã gán thay đổi; tải lại nick rồi đối chiếu.");
      setReading(next);
    } catch (e) { if (ticket.current === request) { setReading(null); setError(describeError(e)); } }
    finally { if (ticket.current === request) setBusy(false); }
  }
  const text = !reading ? "Chưa đối chiếu" : ({ matched: "Khớp tài khoản", mismatch: "Lệch tài khoản", unknown: "Chưa đọc được", unassigned: "Chưa gán nick" }[reading.status] ?? "Chưa đọc được");
  return <div className="interaction-account-proof">
    <button type="button" className="btn btn-sm" disabled={disabled || busy} onClick={() => void check()} title="Mở Hồ sơ trên máy để đọc tài khoản, không đổi nick đã gán"><ScanSearch size={14} aria-hidden="true" />{busy ? "Đang đọc tài khoản…" : "Đọc tài khoản từ máy"}</button>
    <small role="status" className={reading?.status === "mismatch" ? "interaction-error" : undefined}>{text}{reading?.observedHandle ? ` · @${reading.observedHandle}` : ""}</small>
    {reading && <small><time dateTime={reading.checkedAt}>{new Date(reading.checkedAt).toLocaleString("vi-VN")}</time></small>}
    {error && <small role="alert">{error}</small>}
  </div>;
}
