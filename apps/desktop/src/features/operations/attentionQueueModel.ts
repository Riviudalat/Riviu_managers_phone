import type { DeviceOperationalView } from "../../deviceWork";
import type { OperationRunDetail, OperationRunSummary, PublishAssignmentRecord } from "../../types";
import { deviceRows, RUN_STATE_LABEL } from "./operationProgress";
import type { PostOutcome } from "./postOutcomeModel";
import { publishRunOutcomes, type PublishRunRead } from "./publishRunRead";

/**
 * The "Cần xử lý" queue (PLAN §5): what needs the operator, one row per phone and reason.
 *
 * Three sources, each read where the rest of the app reads it, so the queue never keeps a second
 * truth: phones whose condensed status is Cần xử lý (the header counter's own set), posts the
 * shared outcome model leaves Chưa rõ or with a failed Sheet write, and phones a run ended in
 * failure or uncertainty for. Chưa rõ and Sheet errors are `persistent`: they stay until the
 * backend records a conclusion and no dismissal hides them. Nothing here starts work.
 */
export type AttentionKind = "device" | "post" | "sheet" | "publishFailed" | "runItem";

/** A phone as the queue names it: its number, its box and whoever holds it right now. */
export interface QueuePhone {
  machine: string;
  box: string;
  holder: string;
}

export interface DeviceAttention {
  udid: string;
  view: DeviceOperationalView;
}

export interface AttentionItem {
  /** Stable while the reason holds; a new reason is a new item. */
  key: string;
  kind: AttentionKind;
  udid: string | null;
  machine: string;
  box: string;
  /** "Không đạt: Chưa điều khiển được", "Chưa rõ", "Lỗi ghi Sheet"… */
  reason: string;
  detail?: string;
  /** Epoch ms of the backend record, or `null` when only this window's first sight is known. */
  recordedAt: number | null;
  holder: string;
  nextStep: string;
  /** Chưa rõ and Sheet errors: only a backend conclusion removes them. */
  persistent: boolean;
  outcome?: PostOutcome;
  /** Raw run-item evidence shown by "Xem bằng chứng" for non-publish runs. */
  evidence?: string;
}

export interface AttentionGroup {
  key: string;
  title: string;
  run: OperationRunSummary | null;
  items: AttentionItem[];
  /** Why the group's items could not be read; the count is then unknown. */
  error?: string;
  /** The read failed after a success: the items are those of the last successful read. */
  stale?: boolean;
}

export const UNASSIGNED_BOX = "Chưa thuộc box";

/** Who holds the phone right now, from its Công việc axis; unread is said, never "nobody". */
export function queueHolder(view: DeviceOperationalView): string {
  if (view.axes.work.state === "running") return `${view.axes.work.detail ?? "Một tác vụ"} đang giữ máy`;
  if (view.axes.work.state === "unknown") return "Chưa rõ tác vụ đang giữ máy";
  return "Không tác vụ nào giữ máy";
}

export function machineOf(udid: string | null, phones: ReadonlyMap<string, QueuePhone>, labels: ReadonlyMap<string, string>, recorded?: string | null): string {
  if (!udid) return "Toàn tác vụ";
  const phone = phones.get(udid);
  if (phone) return phone.machine;
  // A run keeps the number it recorded when the phone has left the current fleet.
  if (recorded && /^Máy \d+(?:\s|$)/.test(recorded)) return recorded.replace(/\s*·.*$/, "");
  const label = labels.get(udid);
  if (label) return label.replace(/\s*·.*$/, "");
  return `Thiết bị …${udid.slice(-6)}`;
}

function phoneFacts(udid: string | null, phones: ReadonlyMap<string, QueuePhone>) {
  const phone = udid ? phones.get(udid) : undefined;
  return {
    box: phone?.box ?? UNASSIGNED_BOX,
    holder: phone?.holder ?? (udid ? "Máy không còn trong danh sách" : "Không áp dụng"),
  };
}

function time(value: string | null | undefined): number | null {
  if (!value) return null;
  const at = Date.parse(value);
  return Number.isFinite(at) ? at : null;
}

/** The phones of the header counter "Cần xử lý": why each one needs the operator and what to do. */
export function deviceAttentionItems(
  attention: readonly DeviceAttention[],
  phones: ReadonlyMap<string, QueuePhone>,
  labels: ReadonlyMap<string, string>,
): AttentionItem[] {
  return attention.map(({ udid, view }) => {
    const nextStep = view.helperRecovery
      ? "Khôi phục helper: Trung tâm điều khiển › Bảo trì › Khôi phục helper."
      : view.axes.control.state === "failed"
        ? "Mở Chẩn đoán để kiểm tra Agent trên máy này."
        : view.detail === "Máy báo lỗi"
          ? "Mở Chẩn đoán để xem lỗi; cắm lại USB nếu máy vẫn báo lỗi."
          : "Chờ mẫu đo tải mới; nếu máy vẫn quá tải, khởi động lại máy.";
    return {
      key: `device:${udid}:${view.detail ?? ""}`,
      kind: "device" as const,
      udid,
      machine: machineOf(udid, phones, labels),
      ...phoneFacts(udid, phones),
      reason: view.axes.control.state === "failed" ? `Không đạt: ${view.detail ?? "Chưa điều khiển được"}` : view.detail ?? "Cần xử lý",
      detail: view.reason && view.reason !== view.detail ? view.reason : undefined,
      recordedAt: null,
      nextStep,
      persistent: false,
    };
  });
}

function postNextStep(assignment: PublishAssignmentRecord): string {
  return assignment.errorCode === "post_verification_needs_review"
    ? "Tự kiểm tra đã dừng: mở Theo dõi để xem bằng chứng hoặc kiểm tra liên kết. Không đăng lại."
    : "Chỉ theo dõi: Riviu tự kiểm tra link, tài khoản, caption và thời điểm khi máy rảnh. Không đăng lại.";
}

/**
 * A publish run's rows from the shared full-assignment outcomes: the same model, read and
 * numbers as the campaign detail and Lượt chạy.
 */
export function publishAttentionItems(
  read: PublishRunRead,
  run: OperationRunSummary,
  phones: ReadonlyMap<string, QueuePhone>,
  labels: ReadonlyMap<string, string>,
): AttentionItem[] {
  const outcomes = publishRunOutcomes(read);
  const items: AttentionItem[] = [];
  for (const assignment of read.assignments) {
    const outcome = outcomes.get(assignment.id);
    if (!outcome) continue;
    const base = {
      udid: assignment.udid,
      machine: machineOf(assignment.udid, phones, labels),
      ...phoneFacts(assignment.udid, phones),
      outcome,
    };
    const submittedAt = time(outcome.proof.submittedAt) ?? time(outcome.proof.checkedAt);
    if (outcome.uncertain) {
      items.push({
        ...base, key: `post:${assignment.id}`, kind: "post", reason: "Chưa rõ bài đã lên",
        detail: outcome.postReason, recordedAt: submittedAt ?? time(run.updatedAt),
        nextStep: postNextStep(assignment), persistent: true,
      });
    } else if (outcome.sheet === "failed") {
      items.push({
        ...base, key: `sheet:${assignment.id}`, kind: "sheet", reason: "Lỗi ghi Sheet",
        detail: outcome.sheetReason, recordedAt: time(assignment.sheetDelivery?.updatedAt) ?? time(run.updatedAt),
        nextStep: "Sửa nguyên nhân rồi chọn Ghi lại Sheet trong Theo dõi; bài đã xác minh vẫn giữ link.", persistent: true,
      });
    } else if (assignment.state === "failedBeforeDispatch" || assignment.state === "missed") {
      items.push({
        ...base, key: `publishFailed:${assignment.id}`, kind: "publishFailed",
        reason: assignment.state === "missed" ? "Lỡ lịch, chưa bấm Đăng" : "Dừng trước khi bấm Đăng",
        detail: assignment.errorCode ?? undefined, recordedAt: time(run.updatedAt),
        nextStep: "Xem nhật ký trong Theo dõi; bài này chưa được gửi.", persistent: false,
      });
    }
  }
  return items;
}

const ISSUE_STATES = new Set(["failed", "uncertain", "partial"]);

/** A non-publish run's phones that ended in failure or uncertainty. Uncertain sends stay. */
export function runAttentionItems(
  detail: OperationRunDetail,
  run: OperationRunSummary,
  phones: ReadonlyMap<string, QueuePhone>,
  labels: ReadonlyMap<string, string>,
): AttentionItem[] {
  return deviceRows(detail.items)
    .filter(row => row.udid && ISSUE_STATES.has(row.state))
    .map(row => {
      const udid = row.udid;
      const explained = row.entries.find(entry => entry.detail?.trim());
      const uncertain = row.state === "uncertain";
      const evidence = row.entries.map(entry => [entry.errorCode, entry.evidence].filter(Boolean).join("\n")).filter(Boolean).join("\n\n");
      return {
        key: `run:${run.id}:${udid}:${row.state}`,
        kind: "runItem" as const,
        udid,
        machine: machineOf(udid, phones, labels, row.entries[0]?.label),
        ...phoneFacts(udid, phones),
        reason: uncertain ? "Chưa rõ kết quả thao tác" : RUN_STATE_LABEL[row.state],
        detail: explained?.detail?.trim() ?? row.entries.find(entry => entry.errorCode)?.errorCode ?? undefined,
        recordedAt: time(run.updatedAt),
        nextStep: uncertain
          ? "Chỉ theo dõi: mở Theo dõi để xem nhật ký và bằng chứng. Không gửi lại."
          : "Mở Theo dõi để xem nhật ký của máy; chạy lại từ trang của tác vụ nếu cần.",
        persistent: uncertain,
        evidence: evidence || undefined,
      };
    });
}

function machineNumber(item: AttentionItem): number {
  const match = /^Máy (\d+)/.exec(item.machine);
  return match ? Number(match[1]) : Number.MAX_SAFE_INTEGER;
}

/** Fixed order: phones by machine number, never by when their state changed. */
export function sortAttentionItems(items: AttentionItem[]): AttentionItem[] {
  return [...items].sort((left, right) => machineNumber(left) - machineNumber(right) || left.key.localeCompare(right.key));
}
