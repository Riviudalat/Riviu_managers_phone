import { useCallback, useEffect, useState } from "react";

import { nurtureCostSummary, nurtureListCommentAttempts } from "../../api";
import { evidenceLabel } from "../../commentEvidence";
import { describeError } from "../../describeError";
import type { NurtureCommentAttempt, NurtureCostSummary } from "../../types";
import { EmptyState, LoadingState, StatusNotice } from "../States";

/** How many rows to keep on screen. Beyond this the list stops being readable. */
const LIMIT = 60;
/** Slower than the log's 2.5 s: a comment takes seconds of API time, not milliseconds. */
const POLL_MS = 4_000;

/**
 * Every comment the fleet *considered* — sent, rejected by the gate, or skipped.
 *
 * **This table existed for months with nothing rendering it.** The Tauri command
 * (`nurture_list_comment_attempts`) was registered and allowlisted, the row type was mirrored
 * into TypeScript, and `api.ts` never called any of it: the only way to read the audit was the
 * final dump of the `live_nurture_android` binary. So the interesting rows — the skips, which
 * say the evidence was unusable or the verifier refused the draft — were invisible in the app
 * that produced them.
 *
 * The column that forced the issue is `distinctFrames`. It is the number that makes
 * `evidenceSupport` legible, and a number no screen reads cannot be checked against a run.
 */
/**
 * Tokens and comments, today and in total.
 *
 * **Deliberately does not multiply by a price.** The app had a `usd` column once, computed from
 * two rates typed into settings by hand and never sent to the API; three different pairs of them
 * existed in the codebase at once, and after any model change every figure was silently wrong.
 * Migration 11 dropped it. Tokens come from the provider's own `usage` object and are true of
 * whatever model is configured, so they are what this shows -- multiply by the real rate outside
 * the app.
 *
 * Counts **every** attempt, sent or rejected: a comment the verification gate threw away still
 * burned up to four API calls, and reporting that as free is how the most expensive failure mode
 * became invisible.
 */
function CostStrip({ totals }: { totals: NurtureCostSummary }) {
  const tokens = (prompt: number, completion: number) =>
    `${(prompt + completion).toLocaleString("vi-VN")} token`;
  return (
    <dl className="nurture-attempt-totals">
      <div>
        <dt>Hôm nay</dt>
        <dd>
          {totals.todayComments} bình luận · {tokens(totals.todayPromptTokens, totals.todayCompletionTokens)}
        </dd>
      </div>
      <div>
        <dt>Tổng</dt>
        <dd>
          {totals.totalComments} bình luận · {tokens(totals.totalPromptTokens, totals.totalCompletionTokens)}
        </dd>
      </div>
    </dl>
  );
}

export function NurtureCommentsTab({
  live,
  deviceLabel = () => "Máy chưa xác định",
}: {
  /// Poll only while something is running: a finished table does not change on its own.
  live: boolean;
  deviceLabel?: (udid: string) => string;
}) {
  const [rows, setRows] = useState<NurtureCommentAttempt[] | null>(null);
  /**
   * The aggregate over the same table the rows come from.
   *
   * Its own state, and a failure here does **not** take the table down with it: the rows are
   * what an operator is reading, and a broken total is a missing strip rather than a blank
   * panel. Kept `null` on error for that reason.
   */
  const [totals, setTotals] = useState<NurtureCostSummary | null>(null);
  const [totalsError, setTotalsError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    void nurtureListCommentAttempts(LIMIT)
      .then((next) => {
        setRows(next);
        setError(null);
      })
      .catch((cause) => setError(describeError(cause)));
    void nurtureCostSummary()
      .then((next) => {
        setTotals(next);
        setTotalsError(null);
      })
      .catch((cause) => {
        setTotals(null);
        setTotalsError(describeError(cause));
      });
  }, []);

  useEffect(load, [load]);
  // Re-read only while something is running: a finished table does not change on its own, and
  // this panel is often left open for a whole session.
  useEffect(() => {
    if (!live) return;
    const timer = setInterval(load, POLL_MS);
    return () => clearInterval(timer);
  }, [live, load]);

  if (error) {
    return (
      <StatusNotice
        tone="error"
        action={
          <button type="button" className="secondary" onClick={load}>
            Thử lại
          </button>
        }
      >
        {error}
      </StatusNotice>
    );
  }
  if (!rows) {
    return <LoadingState label="Đang đọc lịch sử bình luận…" />;
  }
  if (!rows.length) {
    return (
      <div className="nurture-attempts">
        {totals && <CostStrip totals={totals} />}
        {totalsError && <TotalsError onRetry={load} />}
        <EmptyState
          compact
          title="Chưa có lượt bình luận nào được ghi"
          hint="Lịch sử sẽ xuất hiện khi một phiên đi tới bước soạn bình luận."
        />
      </div>
    );
  }

  return (
    <div className="nurture-attempts">
      {totals && <CostStrip totals={totals} />}
      {totalsError && <TotalsError onRetry={load} />}
      <p className="hint">
        {rows.length} lượt gần nhất · gồm cả lượt bị gate chặn và lượt bỏ qua
      </p>
      <ul className="nurture-attempt-list">
        {rows.map((row) => (
          <li key={row.id} className={`nurture-attempt${outcomeClass(row.outcome)}`}>
            <div className="nurture-attempt-head">
              <strong title={row.udid}>{deviceLabel(row.udid)}</strong>
              <span className="nurture-attempt-outcome" title={row.outcome}>
                {outcomeLabel(row.outcome)}
              </span>
              <span className="grow" />
              <span className="nurture-attempt-when">{shortTime(row.createdAt)}</span>
            </div>
            {row.preview ? (
              <p className="nurture-attempt-text">“{row.preview}”</p>
            ) : (
              row.captionPreview && (
                <p className="nurture-attempt-text is-caption">chú thích: {row.captionPreview}</p>
              )
            )}
            <p className="hint">
              {evidenceLabel(row)}
              {slidesNote(row)}
              {row.evidenceSupport === undefined ? "" : ` · bằng chứng ${row.evidenceSupport}/100`}
              {row.relevance === undefined ? "" : ` · hợp đề ${row.relevance}/100`}
              {` · ${row.promptTokens} token vào / ${row.completionTokens} ra`}
            </p>
          </li>
        ))}
      </ul>
    </div>
  );
}

function TotalsError({ onRetry }: { onRetry: () => void }) {
  return (
    <StatusNotice
      tone="warning"
      action={
        <button type="button" className="secondary" onClick={onRetry}>
          Thử lại
        </button>
      }
    >
      Chưa đọc được tổng chi phí. Danh sách lượt vẫn đầy đủ.
    </StatusNotice>
  );
}

/**
 * How many slides the traversal paged, when it paged any.
 *
 * Only worth a word next to the frame count, which is the pair that says something: seven
 * slides with one distinct frame means the pager turned seven times and the stream handed back
 * the same picture every time. Silent on a post that was never paged — a video, or a build
 * with no measured photo badge — because `0 ảnh` there is noise, not information.
 */
function slidesNote(row: NurtureCommentAttempt): string {
  if (!row.carouselSlides) return "";
  return ` · lướt ${row.carouselSlides} ảnh`;
}

/**
 * `outcome` is free text written by the engine — `sent`, `prepared`, `skipped: …`,
 * `context_skipped: …`, `failed: …` — so this reads its prefix rather than matching a closed
 * set. An unrecognised outcome shows verbatim instead of being flattened into "khác".
 */
function outcomeLabel(outcome: string): string {
  if (outcome === "sent") return "đã gửi";
  if (outcome === "prepared") return "đã soạn, chưa rõ kết quả";
  if (outcome === "skipped: card_changed") return "bỏ — thẻ đã đổi trước thao tác";
  if (outcome === "deferred_card_changed") return "bỏ — thẻ đổi khi đang xem ảnh";
  if (outcome === "deferred_no_rail") return "bỏ — rời khỏi thẻ trước khi kịp gửi";
  if (outcome === "deferred_stopped") return "bỏ — phiên dừng khi đang xem ảnh";
  if (outcome.startsWith("context_skipped")) return "bỏ — bằng chứng không dùng được";
  if (outcome.startsWith("skipped")) return "bỏ — gate chặn";
  if (outcome.startsWith("failed")) return "lỗi khi gửi";
  return "trạng thái chưa nhận diện";
}

function outcomeClass(outcome: string): string {
  if (outcome === "sent") return " is-sent";
  if (outcome.startsWith("failed")) return " is-failed";
  if (outcome === "prepared") return "";
  return " is-skipped";
}

/** `HH:MM` in the operator's own clock; the date is noise on a table this short. */
function shortTime(iso: string): string {
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return iso;
  return at.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}
