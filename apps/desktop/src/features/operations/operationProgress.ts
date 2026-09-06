import { deviceProgress } from "../../nurtureProgress";
import type { NurtureSessionStatus, OperationDeviceLogEntry, OperationRunItem, OperationRunState, OperationRunSummary } from "../../types";
import { ACTION_PRESENTATION } from "../../components/flow/actionPresentation";
import type { ActionKind } from "../../types";

export const RUN_STATE_LABEL: Record<OperationRunState, string> = {
  queued: "Đang chờ", running: "Đang chạy", succeeded: "Hoàn tất", partial: "Hoàn tất một phần",
  failed: "Thất bại", uncertain: "Chưa xác nhận", cancelled: "Đã dừng", skipped: "Đã bỏ qua",
};
export const activeRun = (run: Pick<OperationRunSummary, "state">) => run.state === "queued" || run.state === "running";
export const issueState = (state: OperationRunState) => ["failed", "partial", "uncertain"].includes(state);
export const progressLabel = (fraction: number | null) => fraction === null ? "Chưa rõ %" : `${Math.round(fraction * 100)}%`;

export function runProgress(run: OperationRunSummary, sessions: NurtureSessionStatus[], now = Date.now()): number | null {
  if (!activeRun(run)) return 1;
  if (run.kind === "nurture") {
    const rows = sessions.filter((row) => row.runId === run.sourceId);
    if (rows.length && run.targetCount > 0) {
      return Math.min(.99, rows.reduce((sum, row) => sum + (deviceProgress(row, now) ?? 0), 0) / run.targetCount);
    }
  }
  if (run.totalItems <= 0) return null;
  return Math.min(.99, Math.max(0, run.completedItems / run.totalItems));
}

export function deviceRows(items: OperationRunItem[]) {
  const rows = new Map<string, OperationRunItem[]>();
  for (const item of items) {
    const key = item.udid ?? "";
    rows.set(key, [...(rows.get(key) ?? []), item]);
  }
  return [...rows].map(([udid, entries]) => {
    // Parent device rows already summarize Flow attempts; never count both levels.
    const work = entries.some((item) => item.kind === "device") ? entries.filter((item) => item.kind === "device") : entries;
    const states = new Set(work.map((item) => item.state));
    const state: OperationRunState = states.has("running") ? "running"
      : states.has("queued") ? "queued"
      : states.has("uncertain") ? "uncertain"
      : states.size > 1 ? "partial" : work[0]?.state ?? "queued";
    return { udid, entries, state, fraction: work.length ? work.filter((item) => !activeRun(item)).length / work.length : null };
  });
}

export function deviceStateCounts(rows: readonly { udid: string; state: OperationRunState }[]) {
  const counts = { total: 0, succeeded: 0, issues: 0, active: 0, stopped: 0 };
  const bucket: Record<OperationRunState, "succeeded" | "issues" | "active" | "stopped"> = {
    queued: "active", running: "active", succeeded: "succeeded", partial: "issues",
    failed: "issues", uncertain: "issues", cancelled: "stopped", skipped: "stopped",
  };
  for (const row of rows) {
    if (!row.udid) continue;
    counts.total += 1;
    counts[bucket[row.state]] += 1;
  }
  return counts;
}

export function logTime(at: string | null): string {
  if (!at || Number.isNaN(Date.parse(at))) return "--:--:--";
  return new Date(at).toLocaleTimeString("vi-VN", { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });
}

const LOG_STATE: Record<string, string> = {
  ...RUN_STATE_LABEL, planned: "Đã lên lịch", preparing: "Đang chuẩn bị", ready: "Sẵn sàng",
  armed: "Đã ghi ý định, chờ xác nhận", confirmed: "Đã xác nhận", no_op: "Không cần thao tác",
  failed_before_effect: "Dừng trước thao tác", sending: "Đang gửi", skipped_parent: "Bỏ qua vì lượt trước chưa xác nhận",
  transferring: "Đang chuyển nội dung", imported: "Đã chuyển nội dung", posting: "Đang đăng",
  failedBeforeDispatch: "Dừng trước thao tác", failed_before_dispatch: "Dừng trước thao tác",
  intentCommitted: "Đã ghi ý định", effectDispatched: "Đã gửi thao tác", verifying: "Đang xác nhận",
  failedVerified: "Đã xác nhận thất bại", interrupted: "Bị gián đoạn",
};
const LOG_ACTION: Record<string, string> = { nurture: "Nuôi TikTok", interaction: "Tương tác",
  publish: "Đăng bài", like: "Tim", save: "Lưu bài", comment: "Bình luận", follow: "Theo dõi",
  appInstall: "Cài ứng dụng", materialTransfer: "Chuyển nội dung" };
const EVIDENCE: Record<string, string> = { "tiktok-cleanup": "Đã ghi bằng chứng tắt TikTok",
  "frame": "Đã lưu ảnh kiểm tra", "before": "Đã lưu ảnh trước thao tác", "after": "Đã lưu ảnh sau thao tác" };

export function logMessage(row: OperationDeviceLogEntry): string {
  if (row.action === "nurture" && row.text?.trim()) {
    const text = row.text.trim();
    const exact: Record<string, string> = { queued: "Đang chờ bắt đầu", "bắt đầu": "Bắt đầu phiên", "mở phiên điều khiển mới": "Mở phiên điều khiển mới" };
    if (exact[text]) return exact[text];
    if (text.startsWith("nhãn đã đo:")) return "Đã nhận diện cấu hình TikTok";
    const result = /^(done|partial|failed|stopped)\s*[—-]\s*/.exec(text);
    if (result) {
      const label: Record<string, string> = { done: "Hoàn tất", partial: "Hoàn tất một phần", failed: "Thất bại", stopped: "Đã dừng" };
      return `${label[result[1]]}: ${text.slice(result[0].length).replace(/\s*\(hierarchy\)/g, "")}`;
    }
    return text;
  }
  if (row.action === "evidence") return EVIDENCE[row.state] ?? "Đã lưu bằng chứng kiểm tra";
  const action = LOG_ACTION[row.action] ?? ACTION_PRESENTATION[row.action as ActionKind]?.label ?? "Thao tác";
  return `${action} · ${LOG_STATE[row.state] ?? "Đã cập nhật trạng thái"}`;
}

export function compactLogEntries(entries: OperationDeviceLogEntry[]) {
  const result: { entry: OperationDeviceLogEntry; count: number; lastAt: string | null }[] = [];
  for (const entry of entries) {
    const previous = result.at(-1);
    if (previous && previous.entry.action === entry.action && previous.entry.state === entry.state
      && previous.entry.text === entry.text && previous.entry.detail === entry.detail) {
      previous.count += 1;
      previous.lastAt = entry.at;
    } else result.push({ entry, count: 1, lastAt: entry.at });
  }
  return result;
}

export function monitorDeviceName(label: string): { name: string; model: string | null } {
  const match = /^(Máy \d+)\s*·\s*(.+)$/.exec(label);
  // Only recognizable hardware model labels move to secondary text. User aliases stay primary.
  if (match && /^(SM[ -]|iPhone\d|Pixel \d|Redmi |POCO |Moto )/.test(match[2])) return { name: match[1], model: match[2] };
  return { name: label, model: null };
}

export function runOptionLabel(run: OperationRunSummary): string {
  const at = run.createdAt ?? run.updatedAt;
  const when = at && !Number.isNaN(Date.parse(at))
    ? new Date(at).toLocaleString("vi-VN", { day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit", hour12: false }) : "Chưa có thời gian";
  return `${run.title} · ${run.targetCount} ${run.kind === "interaction" ? "bài" : "máy"} · ${when}`;
}
