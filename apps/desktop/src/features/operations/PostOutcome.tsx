import { CircleCheck, CircleDashed, CircleHelp, CircleX, Hourglass, Minus, Send, type LucideIcon } from "lucide-react";
import type { PublishSheetDeliveryProgress } from "../../types";
import type { PostLayer, PostOutcome, PostOutcomeTotals as Totals } from "./postOutcomeModel";
import { postOutcomeTotalsText } from "./postOutcomeModel";
import "../../styles/post-outcome.css";

type StepState = "done" | "pending" | "unknown" | "failed" | "off";

const STEP_ICONS: Record<StepState, LucideIcon> = {
  done: CircleCheck,
  pending: CircleDashed,
  unknown: CircleHelp,
  failed: CircleX,
  off: Minus,
};
const HEADLINE_ICONS: Record<PostLayer, LucideIcon> = {
  notSent: Minus,
  sending: Send,
  verifying: Hourglass,
  verified: CircleCheck,
  uncertain: CircleHelp,
};

// A layer says "Đã …" only once it is done; an unfinished layer names its own state, so the
// row never reads as done when skimmed, with or without colour.
const SENT_WORDS: Partial<Record<StepState, string>> = { done: "Đã gửi", pending: "Đang đăng", off: "Chưa gửi" };
const VERIFIED_WORDS: Partial<Record<StepState, string>> = { done: "Đã xác minh", pending: "Đang xác minh", unknown: "Chưa rõ xác minh", off: "Chưa xác minh" };
const SHEET_WORDS: Record<StepState, string> = { done: "Đã ghi Sheet", pending: "Chờ ghi Sheet", failed: "Lỗi ghi Sheet", unknown: "Chưa rõ Sheet", off: "Không ghi Sheet" };
// Before the post is verified the Sheet has nothing to write yet.
const SHEET_WAITING = "Sheet chờ xác minh";

function steps(outcome: PostOutcome): { key: string; label: string; state: StepState; title?: string }[] {
  const sent: StepState = outcome.sent ? "done" : outcome.post === "sending" ? "pending" : "off";
  const verified: StepState = outcome.verified ? "done"
    : outcome.uncertain ? "unknown"
      : outcome.post === "verifying" ? "pending" : "off";
  const sheet: StepState = outcome.sheet === "sent" ? "done"
    : outcome.sheet === "failed" ? "failed"
      : outcome.sheet === "unknown" ? "unknown"
        : outcome.sheet === "off" || outcome.sheet === "superseded" ? "off" : "pending";
  return [
    { key: "sent", label: SENT_WORDS[sent] ?? "Chưa gửi", state: sent },
    { key: "verified", label: VERIFIED_WORDS[verified] ?? "Chưa xác minh", state: verified, title: outcome.postReason },
    { key: "sheet", label: outcome.sheet === "superseded" ? "Đợt báo cáo đã đóng" : outcome.sheet === "waiting" ? SHEET_WAITING : SHEET_WORDS[sheet], state: sheet, title: outcome.sheetLabel },
  ];
}

/**
 * The layered result of one post: Đã gửi → Đã xác minh → Đã ghi Sheet, each with its own icon,
 * plus the headline state (Chưa rõ is its own state) and the missing proof. Used by the campaign
 * detail, the operation monitor and Lượt chạy, so every surface says the same thing.
 */
export function PostOutcomeLayers({ outcome, reason }: { outcome: PostOutcome; reason?: string }) {
  const Headline = HEADLINE_ICONS[outcome.post];
  const shownReason = reason ?? outcome.postReason;
  return (
    <div className={`post-outcome is-${outcome.post}`}>
      <span className="post-outcome-headline">
        <Headline size={14} strokeWidth={2.25} aria-hidden="true" />
        {outcome.postLabel}
      </span>
      {outcome.post !== "notSent" && (
        <ol className="post-outcome-layers" aria-label="Các lớp kết quả bài đăng">
          {steps(outcome).map(step => {
            const Icon = STEP_ICONS[step.state];
            return (
              <li key={step.key} data-state={step.state} title={step.title}>
                <Icon size={13} strokeWidth={2.25} aria-hidden="true" />
                {step.label}
              </li>
            );
          })}
        </ol>
      )}
      {shownReason && <p className="post-outcome-reason">{shownReason}</p>}
    </div>
  );
}

/** The verified link, or why there is none. A Sheet error never hides it. */
export function PostOutcomeLink({ outcome }: { outcome: PostOutcome }) {
  if (outcome.url) {
    return <a href={outcome.url} target="_blank" rel="noreferrer" aria-label="Mở bài đã xác nhận">Đã xác minh · Mở bài</a>;
  }
  return <span>{outcome.uncertain ? "Chưa có link đã xác minh; không tự đăng lại"
    : outcome.post === "verifying" ? "Đã gửi; đang xác minh link, tài khoản, caption và thời điểm"
      : outcome.post === "sending" ? "Đang đăng; chưa có link"
        : "Chưa có link đã xác minh"}</span>;
}

/** The Sheet layer with its delivery attempts. */
export function PostOutcomeSheet({ outcome, delivery }: { outcome: PostOutcome; delivery?: PublishSheetDeliveryProgress | null }) {
  const live = delivery && delivery.state !== "sent" && delivery.state !== "superseded";
  const next = live && delivery.nextAttemptAtMs != null ? new Date(delivery.nextAttemptAtMs).toLocaleTimeString("vi-VN") : null;
  return <span className={`post-outcome-sheet is-${outcome.sheet}`}>
    {outcome.sheetLabel}
    {outcome.sheetReason && <p>{outcome.sheetReason}</p>}
    {live && delivery.attempts > 0 && <p>Đã thử {delivery.attempts} lần · Gần nhất: {new Date(delivery.updatedAt).toLocaleTimeString("vi-VN")}</p>}
    {next && <p>Thử tiếp: {next}</p>}
    {live && delivery.nextAttemptAtMs == null && <p>Sửa nguyên nhân rồi chọn Ghi lại Sheet</p>}
  </span>;
}

function time(value: string | null): string | null {
  return value && Number.isFinite(Date.parse(value)) ? new Date(value).toLocaleString("vi-VN") : null;
}

/** "Xem bằng chứng": what the backend recorded for this post, readable first, raw last. */
export function PostOutcomeEvidence({ outcome, raw = true }: { outcome: PostOutcome; raw?: boolean }) {
  const { proof } = outcome;
  const rawText = raw ? proof.raw : null;
  const match = (value: boolean | null) => value === null ? "Chưa rõ" : value ? "Khớp" : "Chưa khớp";
  const rows = [
    proof.method && ["Cách xác minh", proof.method],
    proof.account && ["Tài khoản của lượt", proof.account],
    time(proof.submittedAt) && ["Bấm Đăng lúc", time(proof.submittedAt)],
    // Shown for every outcome: a recorded mismatch is the reason a row is not verified.
    proof.captionMatched !== null && ["Caption", match(proof.captionMatched)],
    proof.timeMatched !== null && ["Thời điểm cấp mã bài", match(proof.timeMatched)],
    !outcome.verified && proof.candidateUrl && ["Link ứng viên (chưa xác minh)", proof.candidateUrl],
    time(proof.checkedAt) && ["Kiểm tra gần nhất", time(proof.checkedAt)],
  ].filter((row): row is [string, string] => Array.isArray(row));
  if (!rows.length && !rawText) return null;
  return <details className="post-outcome-evidence">
    <summary>Xem bằng chứng</summary>
    {rows.length > 0 && <dl>{rows.map(([label, value]) => <div key={label}><dt>{label}</dt><dd>{value}</dd></div>)}</dl>}
    {rawText && <details className="publish-technical-details"><summary>Dữ liệu gốc</summary><code>{rawText}</code></details>}
  </details>;
}

/** "20 đã gửi · 18 đã xác minh · 17 đã ghi Sheet · 2 chưa rõ": separate numbers, never one success sum. */
export function PostOutcomeTotals({ totals }: { totals: Totals }) {
  return <p className="post-outcome-totals" role="status" aria-label="Kết quả bài đăng">
    {postOutcomeTotalsText(totals)}
  </p>;
}

/** The permanent explanation of the post-result layers (Trợ giúp). */
export function PostOutcomeLegend() {
  return <div className="device-status-legend">
    <p>Mỗi bài đăng có ba lớp riêng, và <strong>Chưa rõ</strong> là một trạng thái riêng:</p>
    <ul>
      <li><strong>Đã gửi</strong>: Riviu đã bấm Đăng. Gửi lệnh chưa phải là thành công.</li>
      <li><strong>Đã xác minh</strong>: chỉ khi bằng chứng của đúng lượt này chứng minh link bài chuẩn, đúng tài khoản, đúng caption và thời điểm: mã bài được cấp trong khoảng thời gian của lượt (sau lúc chuẩn bị), không phải giờ TikTok công bố bài. Thiếu hoặc mâu thuẫn bằng chứng thì là Đang xác minh hoặc Chưa rõ kèm lý do.</li>
      <li><strong>Đã ghi Sheet</strong>: Google Sheet đã xác nhận dòng của bài đã xác minh. Lỗi Sheet không hạ bài đã xác minh và vẫn giữ link.</li>
    </ul>
    <p>Tổng hợp luôn là các số riêng, ví dụ "20 đã gửi · 18 đã xác minh · 17 đã ghi Sheet · 2 chưa rõ". Bài Chưa rõ chỉ có Xem bằng chứng và Theo dõi (kiểm tra liên kết); không bao giờ có nút đăng lại.</p>
  </div>;
}
