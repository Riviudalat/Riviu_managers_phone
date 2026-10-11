import { useEffect, useState } from "react";
import { LoaderCircle } from "lucide-react";
import { ProgressBar } from "../ProgressBar";

/** The current backend reports a shared phase, not individual check counts. */
export function PublishPreparationStatus({ stage, rows }: {
  stage?: "preparing" | "checking" | null;
  rows: { id: string; name: string; machine: string }[];
}) {
  const [startedAt] = useState(() => Date.now());
  const [now, setNow] = useState(startedAt);
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  return <section className="pw-preflight-pending" aria-label="Chuẩn bị đợt đăng">
    <div className="pw-preflight-phase" role="status">
      <LoaderCircle size={18} aria-hidden="true"/>
      <div><strong>{stage === "preparing" ? "Đang chuẩn bị thiết bị…" : "Đang kiểm tra từng máy…"}</strong>
        <span>{rows.length} bài · {Math.max(0, Math.floor((now - startedAt) / 1000))} giây · Chưa đăng bài</span></div>
    </div>
    <ProgressBar label="Tiến độ chuẩn bị đợt đăng" fraction={null} tone="run"/>
    <p>Đang chờ kết quả kiểm tra; chưa có số đo tiến độ từng máy.</p>
    <div className="pw-preflight-pending-list" role="region" aria-label="Máy đang chờ kết quả kiểm tra" tabIndex={0}>
      {rows.map(row => <div key={row.id}><strong>{row.machine}</strong><span>{row.name}</span><small>Chờ kết quả</small></div>)}
    </div>
  </section>;
}
