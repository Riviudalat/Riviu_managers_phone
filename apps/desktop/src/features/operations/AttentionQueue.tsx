import { useEffect, useState } from "react";
import { CircleHelp, CircleX, Eye, FileSpreadsheet, Inbox, Info, TriangleAlert, Undo2, type LucideIcon } from "lucide-react";
import { EmptyState } from "../../components/States";
import type { OperationRunSummary } from "../../types";
import type { AttentionGroup, AttentionItem, AttentionKind } from "./attentionQueueModel";
import { MonitorReadError } from "./OperationRunDevices";
import { PostOutcomeEvidence, PostOutcomeLayers, PostOutcomeLink } from "./PostOutcome";
import { PublishRunOutcomeTotals } from "./PublishRunOutcome";
import "../../styles/attention-queue.css";

type Tone = "attention" | "unknown" | "error";

function toneOf(item: AttentionItem): Tone {
  if (item.kind === "post" || (item.kind === "runItem" && item.persistent)) return "unknown";
  if (item.kind === "sheet" || item.kind === "runItem") return "error";
  return "attention";
}

const ICONS: Record<Tone, LucideIcon> = { attention: TriangleAlert, unknown: CircleHelp, error: CircleX };
const KIND_ICONS: Partial<Record<AttentionKind, LucideIcon>> = { sheet: FileSpreadsheet };

function clock(at: number, now: number): string {
  const date = new Date(at);
  const today = new Date(now).toDateString() === date.toDateString();
  return today
    ? date.toLocaleTimeString("vi-VN", { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false })
    : date.toLocaleString("vi-VN", { day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit", hour12: false });
}

function relative(at: number, now: number): string {
  const minutes = Math.floor(Math.max(0, now - at) / 60_000);
  if (minutes < 1) return "vừa xong";
  if (minutes < 60) return `${minutes} phút trước`;
  const hours = Math.floor(minutes / 60);
  return hours < 24 ? `${hours} giờ trước` : `${Math.floor(hours / 24)} ngày trước`;
}

/** "3 phút trước · 21:15:02": relative and absolute, and says when only this window's first sight is known. */
function Since({ at, observed, now }: { at: number | null; observed: boolean; now: number }) {
  if (at === null) return <span>Chưa rõ thời điểm</span>;
  return (
    <time dateTime={new Date(at).toISOString()} title={observed
      ? "Thời điểm cửa sổ này thấy máy cần xử lý; trạng thái có thể bắt đầu sớm hơn."
      : new Date(at).toLocaleString("vi-VN")}>
      {observed ? "Thấy từ " : ""}{relative(at, now)} · {clock(at, now)}
    </time>
  );
}

function QueueItem({ item, run, seenAt, now, onWatch, onDeviceDetails }: {
  item: AttentionItem;
  run: OperationRunSummary | null;
  seenAt: (key: string) => number | null;
  now: number;
  onWatch: (run: OperationRunSummary) => void;
  onDeviceDetails?: (udid: string) => void;
}) {
  const tone = toneOf(item);
  const Icon = KIND_ICONS[item.kind] ?? ICONS[tone];
  const at = item.recordedAt ?? seenAt(item.key);
  return (
    <li className={`queue-item tone-${tone}`} aria-label={`${item.machine} · ${item.box} · ${item.reason}`}>
      <div className="queue-item-head">
        <Icon className="queue-item-icon" size={16} aria-hidden="true" />
        <strong className="queue-item-machine">{item.machine}</strong>
        <span className="queue-item-box">{item.box}</span>
        {item.persistent && <span className="queue-item-stay" title="Mục này ở lại tới khi Riviu ghi nhận kết luận; không bỏ qua được.">Ở lại tới khi có kết luận</span>}
      </div>
      {item.outcome
        ? <PostOutcomeLayers outcome={item.outcome} reason={item.kind === "sheet" ? item.detail : undefined} />
        : <p className="queue-item-reason"><strong>{item.reason}</strong>{item.detail && <span>{item.detail}</span>}</p>}
      {item.kind === "sheet" && item.outcome && <p className="queue-item-link"><PostOutcomeLink outcome={item.outcome} /></p>}
      <dl className="queue-item-facts">
        <div><dt>Từ</dt><dd><Since at={at} observed={item.recordedAt === null} now={now} /></dd></div>
        <div><dt>Đang giữ máy</dt><dd>{item.holder}</dd></div>
        <div><dt>Bước tiếp</dt><dd>{item.nextStep}</dd></div>
      </dl>
      <div className="queue-item-actions">
        {item.outcome && (Object.values(item.outcome.proof).some(value => value !== null)
          ? <PostOutcomeEvidence outcome={item.outcome} />
          : <details className="post-outcome-evidence">
            <summary>Xem bằng chứng</summary>
            <p>Chưa có bằng chứng nào được ghi cho bài này. Mở Theo dõi để xem nhật ký của máy.</p>
          </details>)}
        {!item.outcome && item.evidence && <details className="post-outcome-evidence">
          <summary>Xem bằng chứng</summary>
          <pre>{item.evidence}</pre>
        </details>}
        {run && <button type="button" className="ghost" onClick={() => onWatch(run)}><Eye size={14} aria-hidden="true" />Theo dõi</button>}
        {item.kind === "device" && item.udid && onDeviceDetails && (
          <button type="button" className="ghost" onClick={() => onDeviceDetails(item.udid as string)}><Info size={14} aria-hidden="true" />Chi tiết máy</button>
        )}
      </div>
    </li>
  );
}

/**
 * Tab "Cần xử lý": one row per phone and reason, grouped by where it came from. Post rows use
 * the shared post-outcome component and the campaign's totals, so this tab, the campaign detail
 * and Lượt chạy show the same numbers. Chưa rõ and Sheet errors only offer Xem bằng chứng and
 * Theo dõi; nothing here re-posts, re-sends, takes a phone or starts work.
 */
export function AttentionQueueList({ groups, total, unknown, omitted, runsError, retryRuns, attentionUnknown, seenAt, onWatch, onDeviceDetails, onDismiss, undoCount, onUndo, dismissError, onOpenRuns }: {
  groups: AttentionGroup[];
  total: number;
  unknown: boolean;
  omitted: number;
  runsError: string | null;
  retryRuns: () => void;
  attentionUnknown: boolean;
  seenAt: (key: string) => number | null;
  onWatch: (run: OperationRunSummary) => void;
  onDeviceDetails?: (udid: string) => void;
  onDismiss: (run: OperationRunSummary) => void;
  undoCount: number;
  onUndo: () => void;
  dismissError: string | null;
  onOpenRuns?: () => void;
}) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, []);
  return (
    <div className="queue-list">
      {runsError && <MonitorReadError message={`Chưa đọc được lượt chạy: ${runsError}`} retry={retryRuns} />}
      {attentionUnknown && <p className="queue-note" role="status">Chưa đọc đủ trạng thái máy nên số máy cần xử lý chưa rõ; không coi là không có.</p>}
      {dismissError && <p className="queue-note is-error" role="alert">Chưa lưu được danh sách đã xem: {dismissError}</p>}
      {undoCount > 0 && <div className="queue-undo" role="status">
        <span>Đã bỏ {undoCount} mục lỗi khỏi hàng đợi; Chưa rõ và lỗi Sheet vẫn ở lại.</span>
        <button type="button" className="ghost" onClick={onUndo}><Undo2 size={14} aria-hidden="true" />Hoàn tác</button>
      </div>}
      {!groups.length && !unknown && (
        <EmptyState compact icon={<Inbox size={20} />} title="Không có mục cần xử lý"
          hint="Máy cần xử lý, bài đăng Chưa rõ, lỗi ghi Sheet và máy lỗi trong lượt chạy sẽ hiện ở đây. Hàng đợi không tự mở." />
      )}
      {!groups.length && unknown && !runsError && <p className="queue-note" role="status">Đang đọc các nguồn của hàng đợi…</p>}
      {groups.map(group => {
        const failures = group.items.filter(item => !item.persistent).length;
        return (
          <section key={group.key} className="queue-group" aria-label={group.title}>
            <header className="queue-group-head">
              <h3>{group.title} <span className="queue-group-count">· {group.items.length} mục</span></h3>
              {group.run && failures > 0 && (
                <button type="button" className="ghost" aria-label={`Đã xem các mục lỗi của ${group.run.title}`}
                  title="Bỏ các mục lỗi của lượt này khỏi hàng đợi trên máy tính này. Chưa rõ và lỗi Sheet vẫn ở lại; Lượt chạy vẫn giữ bản ghi."
                  onClick={() => group.run && onDismiss(group.run)}>Đã xem</button>
              )}
            </header>
            {group.run?.kind === "publish" && (group.stale || !group.error) && <div className="queue-group-totals"><PublishRunOutcomeTotals campaignId={group.run.sourceId} /></div>}
            {!group.run && <p className="queue-group-note">Cùng tập với bộ đếm Cần xử lý trên thanh hệ thống.</p>}
            {group.error && <p className="queue-note is-error" role="alert">{group.stale
              ? `Chưa đọc lại được lượt này: ${group.error}. Đang giữ các mục của lần đọc trước; chúng chưa có kết luận.`
              : `Chưa đọc được các mục của lượt này: ${group.error}`}</p>}
            {group.items.length > 0 && <ul className="queue-items">
              {group.items.map(item => <QueueItem key={item.key} item={item} run={group.run} seenAt={seenAt} now={now}
                onWatch={onWatch} onDeviceDetails={onDeviceDetails} />)}
            </ul>}
          </section>
        );
      })}
      <footer className="queue-footer">
        <p>
          {unknown ? `Ít nhất ${total} mục: còn nguồn chưa đọc, đọc lỗi hoặc còn trang chưa đọc. ` : total > 0 ? `${total} mục. ` : ""}
          Bài Chưa rõ và lỗi Sheet ở lại tới khi Riviu ghi nhận kết luận, dù lượt cũ đến đâu.
          {omitted > 0 ? ` Còn ít nhất ${omitted} lượt chưa đọc tới; đang đọc dần.` : ""} Lượt đã xong nằm trong Lượt chạy.
        </p>
        {onOpenRuns && <button type="button" className="ghost" onClick={onOpenRuns}>Mở Lượt chạy</button>}
      </footer>
    </div>
  );
}
