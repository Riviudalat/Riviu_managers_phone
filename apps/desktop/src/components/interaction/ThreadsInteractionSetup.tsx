import { useState } from "react";
import type { DeviceInfo } from "../../types";
import { readFormDraft, writeFormDraft } from "../../formDraftStorage";
import { checkThreadsPlan, decodeThreadsDraft, THREADS_ACTIONS, type ThreadsAction, type ThreadsInteractionDraft, type ThreadsInteractionRow } from "../../threadsInteractionPlan";
import "../../styles/threads-interaction.css";

const DRAFT_KEY = "threads-interaction";

/** Draft only. Never route Threads targets into interactionStartThread (TikTok). */
export function ThreadsInteractionSetup({ devices, labels }: { devices: DeviceInfo[]; labels: Map<string, string> }) {
  const [draft, setDraft] = useState<ThreadsInteractionDraft>(() => readFormDraft(DRAFT_KEY, decodeThreadsDraft) ?? { rows: [], runAt: "" });
  const [report, setReport] = useState<ReturnType<typeof checkThreadsPlan> | null>(null);
  const [notice, setNotice] = useState("");
  const edit = (next: ThreadsInteractionDraft) => {
    setDraft(next); setReport(null); setNotice("");
    try { writeFormDraft(DRAFT_KEY, next); }
    catch { setNotice("Không lưu được bản nháp trên máy này. Giữ trang mở để tránh mất nội dung."); }
  };
  const patch = (id: string, values: Partial<ThreadsInteractionRow>) => edit({ ...draft, rows: draft.rows.map(row => row.id === id ? { ...row, ...values } : row) });
  return <details className="threads-interaction-draft">
    <summary>Tương tác Threads · bản nháp riêng</summary>
    <section aria-label="Thiết lập tương tác Threads">
      <p>Mỗi dòng gán một máy/tài khoản cho một link bài và một hành động. Dán link bài của bạn hoặc bài khác; nhập nguyên nội dung muốn gửi.</p>
      <p role="status">Chưa hỗ trợ chạy hoặc lưu lịch Threads: cần nhận diện giao diện và xác minh kết quả trên thiết bị. Giờ bên dưới chỉ lưu trong bản nháp.</p>
      {draft.rows.map((row, index) => <fieldset key={row.id}>
        <legend>Tương tác Threads {index + 1}</legend>
        <label>Máy thực hiện<select value={row.udid} onChange={e => patch(row.id, { udid: e.target.value })}>
          <option value="">Chọn Android</option>
          {row.udid && !devices.some(d => d.udid === row.udid) && <option value={row.udid}>Máy ngoài phạm vi</option>}
          {devices.map(d => <option key={d.udid} value={d.udid} disabled={d.platform !== "android" || d.status !== "ready"}>{labels.get(d.udid) ?? d.name}{d.platform !== "android" || d.status !== "ready" ? " · Chưa sẵn sàng" : ""}</option>)}
        </select></label>
        <label>Username Threads<input value={row.account} placeholder="@taikhoan" onChange={e => patch(row.id, { account: e.target.value })}/></label>
        <label>Link bài Threads<input type="url" value={row.url} placeholder="https://www.threads.com/@taikhoan/post/..." onChange={e => patch(row.id, { url: e.target.value })}/></label>
        <label>Hành động<select value={row.action} onChange={e => patch(row.id, { action: e.target.value as ThreadsAction })}>
          {Object.entries(THREADS_ACTIONS).map(([value, label]) => <option key={value} value={value}>{label}</option>)}
        </select></label>
        <label>Nội dung trả lời hoặc trích dẫn<textarea rows={3} value={row.text} onChange={e => patch(row.id, { text: e.target.value })}/></label>
        <button type="button" onClick={() => edit({ ...draft, rows: draft.rows.filter(r => r.id !== row.id) })}>Xóa dòng {index + 1}</button>
      </fieldset>)}
      <button type="button" disabled={draft.rows.length >= 100} onClick={() => edit({ ...draft, rows: [...draft.rows, { id: crypto.randomUUID(), udid: "", account: "", url: "", action: "reply", text: "" }] })}>Thêm dòng Threads</button>
      <label>Giờ dự kiến (bản nháp)<input type="datetime-local" value={draft.runAt} onChange={e => edit({ ...draft, runAt: e.target.value })}/></label>
      <button type="button" onClick={() => setReport(checkThreadsPlan(draft, devices.filter(d => d.platform === "android" && d.status === "ready").map(d => d.udid)))}>Kiểm tra bản nháp Threads</button>
      {notice && <p role="alert">{notice}</p>}
      {report && <div role="status">{report.issues.length ? <ul>{report.issues.map((issue, i) => <li key={i}>{issue}</li>)}</ul> : <p>{report.rows.length} dòng hợp lệ về đầu vào. Chưa kiểm chứng tài khoản trên máy; chưa tạo lịch hoặc gửi tương tác.</p>}</div>}
    </section>
  </details>;
}
