import { ProgressBar } from "../ProgressBar";
import { useState } from "react";
import { DetailDrawer } from "../WorkspacePrimitives";
import { Banner } from "../States";
import {
  assignmentStateVi,
  campaignStateVi,
  interactionErrorVi,
  stateTone,
} from "../../interactionErrors";
import { timeAgoVi } from "../../timeAgo";
import type { InteractionArtifactRecord } from "../../api";
import { PublicCleanupControl } from "./PublicCleanupControl";
import { InteractionReadbackControl } from "./InteractionReadbackControl";
import type {
  DeviceInfo,
  InteractionActionCounters,
  InteractionActionKind,
  InteractionActionState,
  InteractionAssignmentRecord,
  InteractionCampaignDetail,
  InteractionTargetNote,
  PublicActionResult,
} from "../../types";

const ACTION_KIND_VI: Record<InteractionActionKind, string> = {
  like: "Tim",
  save: "Lưu",
  comment: "Bình luận",
  follow: "Theo dõi",
};

const ACTION_STATE_VI: Record<InteractionActionState, string> = {
  planned: "Đang chờ",
  preparing: "Đang chuẩn bị",
  armed: "Đã ghi ý định, chờ xác nhận",
  confirmed: "Đã xác nhận",
  noOp: "Không cần làm",
  failedBeforeEffect: "Chưa thực hiện",
  uncertain: "Chưa chắc kết quả",
};

function actionTone(state: InteractionActionState): "ok" | "warn" | "danger" | "info" {
  if (state === "confirmed" || state === "noOp") return "ok";
  if (state === "armed" || state === "uncertain") return "warn";
  if (state === "failedBeforeEffect") return "danger";
  return "info";
}

function isBlockedAction(assignment: InteractionAssignmentRecord, action: PublicActionResult): boolean {
  // Older runs could stop the parent without settling its unclaimed child actions.
  return ["failed", "uncertain", "skippedParent"].includes(assignment.state)
    && action.state === "planned"
    && action.effectIntent === null;
}

function assignmentReason(assignment: InteractionAssignmentRecord): string | null {
  const original = assignment.errorCode;
  if (!original || !["Like không an toàn để tiếp tục assignment", "Save không an toàn để tiếp tục assignment"].includes(original.trim())) {
    return original;
  }
  return assignment.actions?.find((action) =>
    ["failedBeforeEffect", "uncertain"].includes(action.state)
    && action.error?.trim()
    && action.error !== original,
  )?.error ?? original;
}

function actionView(action: PublicActionResult, assignment: InteractionAssignmentRecord): { label: string; tone: "ok" | "warn" | "danger" | "info" } {
  if (isBlockedAction(assignment, action)) return { label: "Chưa thực hiện: lượt đã dừng", tone: "danger" };
  if (action.state !== "noOp") return { label: ACTION_STATE_VI[action.state] ?? "Chưa nhận diện trạng thái", tone: actionTone(action.state) };
  let verdict: unknown;
  try { verdict = JSON.parse(action.evidence ?? "null")?.verdict; } catch { /* Evidence remains available in details. */ }
  if (verdict === "alreadyLiked") return { label: "Đã tim từ trước", tone: "ok" };
  if (verdict === "alreadySaved") return { label: "Đã lưu từ trước", tone: "ok" };
  if (verdict === "stateUnreadable") return { label: "Bỏ qua: chưa đọc được trạng thái", tone: "warn" };
  if (verdict === "noControl") return { label: "Bỏ qua: không thấy nút", tone: "warn" };
  if (verdict === "cardChangedBeforeEffect") return { label: "Bỏ qua: bài đã đổi", tone: "warn" };
  return { label: "Không thao tác", tone: "info" };
}

function actionAggregateVi(
  aggregate: NonNullable<InteractionCampaignDetail["actionAggregate"]>,
): string {
  switch (aggregate) {
    case "done":
      return "Hành động hoàn tất";
    case "partial":
      return "Hành động xong một phần";
    case "failed":
      return "Hành động chưa thực hiện";
    case "uncertain":
      return "Hành động chưa chắc kết quả";
  }
}

function actionAggregateTone(
  aggregate: NonNullable<InteractionCampaignDetail["actionAggregate"]>,
): "ok" | "warn" | "danger" {
  if (aggregate === "done") return "ok";
  if (aggregate === "failed") return "danger";
  return "warn";
}

function ActionCounters({
  counters,
  failedBeforeEffect,
}: {
  counters: InteractionActionCounters;
  failedBeforeEffect: number;
}) {
  return (
    <section className="interaction-action-counters" aria-label="Tổng hợp hành động">
      <span aria-label={`${counters.planned} dự kiến`}><strong>{counters.planned}</strong> dự kiến</span>
      <span aria-label={`${counters.attempted} đã thao tác`}><strong>{counters.attempted}</strong> đã thao tác</span>
      <span aria-label={`${counters.confirmed} xác nhận`}><strong>{counters.confirmed}</strong> xác nhận</span>
      <span aria-label={`${counters.noOp} không cần làm`}><strong>{counters.noOp}</strong> không cần làm</span>
      {failedBeforeEffect > 0 && (
        <span className="is-failed" aria-label={`${failedBeforeEffect} chưa thực hiện`}>
          <strong>{failedBeforeEffect}</strong> chưa thực hiện
        </span>
      )}
      {counters.uncertain > 0 && (
        <span className="is-uncertain" aria-label={`${counters.uncertain} chưa chắc`}><strong>{counters.uncertain}</strong> chưa chắc</span>
      )}
    </section>
  );
}

/** One recorded reason, in Vietnamese, with the code kept for whoever needs it. */
function Reason({ code }: { code: string }) {
  const view = interactionErrorVi(code);
  return (
    <span className="interaction-error">
      <strong>{view.title}</strong>
      {/* Never thrown away. The Vietnamese is for the operator; the code is what a bug
          report is written from, and this panel has both kinds of reader. */}
      {view.title !== view.raw && (
        <details className="interaction-raw-code" aria-label="Chi tiết mã lỗi tương tác">
          <summary>mã lỗi</summary>
          {view.detail && <small>{view.detail}</small>}
          <code>{view.raw}</code>
        </details>
      )}
    </span>
  );
}

/** Why a lookup produced nothing, in the operator's language. */
function lookupReasonVi(code: string): string {
  switch (code) {
    case "ip_blocked":
      return "TikTok chặn IP máy này với bài đó — máy trong fleet vẫn xem được";
    case "post_unavailable":
      return "bài không truy cập được (đã xoá, riêng tư, hoặc không phải bài)";
    case "no_ytdlp":
      return "máy này chưa có yt-dlp — xem sidecars/yt-dlp/README.md";
    case "transient":
      return "lỗi tạm thời, đã thử lại 3 lượt";
    default:
      return code;
  }
}

/**
 * What the desktop learned about each target before any phone was touched.
 *
 * **This panel is the point of the column.** AGENTS.md 9.103 §4: the comment audit sat in the
 * database for months because nothing rendered it, so the numbers that made a run legible were
 * unreadable in the app that produced them. `interaction_targets.context_json` would have gone
 * the same way.
 *
 * The three states it has to keep apart, and none of them is "empty":
 *
 * - **enriched** — a caption length, a slide count, maybe a transcript track;
 * - **refused** — `errorCode`, which on this farm is two targets in seven (`ip_blocked`);
 * - **not looked up** — a campaign that ran before this existed, or one that is all manual.
 */
function TargetNotesPanel({ notes }: { notes: InteractionTargetNote[] }) {
  if (notes.length === 0) return null;
  const looked = notes.filter((note) => !isBlankNote(note)).length;
  return (
    <section className="interaction-notes">
      <h4>
        Tra từ web <small>{looked}/{notes.length} bài tra được</small>
      </h4>
      <table className="interaction-notes-table">
        <thead>
          <tr>
            <th>#</th>
            <th>Loại</th>
            <th>Chú thích</th>
            <th>Ảnh</th>
            <th>Dài</th>
            <th>Lời thoại</th>
          </tr>
        </thead>
        <tbody>
          {notes.map((note) => (
            <tr key={note.targetKey} className={note.errorCode ? "is-refused" : undefined}>
              <td>{note.lineNo}</td>
              <td>{note.kind === "photo" ? "ảnh" : "video"}</td>
              <td>
                {note.errorCode ? (
                  <span className="interaction-note-refused">
                    {lookupReasonVi(note.errorCode)}
                  </span>
                ) : note.captionChars === null ? (
                  <span className="interaction-note-blank">chưa tra</span>
                ) : (
                  <>
                    <strong>{note.captionChars} ký tự</strong>
                    {note.captionPreview && <small>{note.captionPreview}…</small>}
                  </>
                )}
              </td>
              {/* A dash, not a zero. `slideCount` is null for every video, and "0 ảnh" beside
                  each of them is a number that means nothing. */}
              <td>{note.slideCount === null ? "—" : `${note.slideCount} ảnh`}</td>
              <td>{note.durationSecs === null ? "—" : `${note.durationSecs}s`}</td>
              <td>
                {note.transcriptTrack ? (
                  <strong>{note.transcriptTrack}</strong>
                ) : note.hasOriginalAudio === false ? (
                  /* The measured reason, not a shrug: the post carries music, so there is no
                     speech to transcribe and no request was spent asking. */
                  <span className="interaction-note-blank">nhạc nền</span>
                ) : (
                  "—"
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

/** Mirrors `InteractionTargetNote::is_blank` — the three states above turn on it. */
function isBlankNote(note: InteractionTargetNote): boolean {
  return (
    note.captionChars === null &&
    note.slideCount === null &&
    note.errorCode === null &&
    note.subtitleLangs.length === 0
  );
}

export function InteractionCampaignDetailView({
  detail,
  artifacts,
  notes,
  devices,
  deviceNumber,
  deviceLabel,
  evidenceError,
  onRetryEvidence,
  handles,
  busy,
  error,
  onBack,
  onCancel,
  onRetry,
  onShowShot,
  shot,
  onDismissShot,
  compact = false,
}: {
  detail: InteractionCampaignDetail;
  artifacts: InteractionArtifactRecord[];
  notes: InteractionTargetNote[];
  devices: DeviceInfo[];
  deviceNumber: Map<string, number>;
  deviceLabel?: Map<string, string>;
  evidenceError?: string | null;
  onRetryEvidence?: () => void;
  handles: Record<string, string>;
  busy: boolean;
  error: string | null;
  onBack: () => void;
  onCancel: () => void;
  onRetry: (assignmentIds?: string[]) => void;
  onShowShot: (artifactId: string) => void;
  shot: string | null;
  onDismissShot: () => void;
  compact?: boolean;
}) {
  const [selectedActor, setSelectedActor] = useState<string | null>(null);
  const { summary } = detail;
  const total = detail.scriptedConversation ? detail.assignments.length : summary.messageCount * summary.targetCount;
  const settled = summary.succeededMessages + summary.failedMessages;
  const actionCounters = summary.actionCounters;
  const hasActionCounters = Boolean(actionCounters?.planned);
  const actionFailedBeforeEffect = detail.assignments.reduce(
    (count, assignment) =>
      count + (assignment.actions ?? []).filter((action) =>
        action.state === "failedBeforeEffect" || isBlockedAction(assignment, action),
      ).length,
    0,
  );
  const actionSettled = actionCounters
    ? actionCounters.confirmed +
      actionCounters.noOp +
      actionCounters.uncertain +
      actionFailedBeforeEffect
    : 0;

  /// A phone by the number on its tile, not by eight characters of its udid.
  ///
  const departedUdids = Array.from(
    new Set(
      detail.assignments
        .map((assignment) => assignment.actorUdid)
        .filter(
          (udid) =>
            !devices.some((device) => device.udid === udid) && !deviceNumber.has(udid),
        ),
    ),
  );
  const departedNumber = new Map(departedUdids.map((udid, index) => [udid, index + 1]));
  const actorLabel = (udid: string) => {
    const number = deviceNumber.get(udid);
    const device = devices.find((entry) => entry.udid === udid);
    const handle = handles[udid];
    if (!device && number === undefined) {
      const departed = departedNumber.get(udid) ?? 1;
      return `Máy đã rời fleet ${departed}/${departedUdids.length}`;
    }
    const name = deviceLabel?.get(udid) || device?.name || device?.model || "Thiết bị chưa đặt tên";
    return `${number ? `${number} · ` : ""}${name}${handle ? ` · @${handle}` : ""}`;
  };

  const byLink = detail.assignments.reduce<Record<string, typeof detail.assignments>>(
    (groups, assignment) => {
      if (compact && selectedActor !== assignment.actorUdid) return groups;
      (groups[assignment.targetKey] ??= []).push(assignment);
      return groups;
    },
    {},
  );

  const byActor = new Map<string, InteractionAssignmentRecord[]>();
  for (const assignment of detail.assignments) {
    const rows = byActor.get(assignment.actorUdid) ?? [];
    rows.push(assignment);
    byActor.set(assignment.actorUdid, rows);
  }

  const evidenceContent = <>
      {detail.conversationSession && <section aria-label="Tiến độ khung giờ hội thoại" className="interaction-thread">
        <strong>Khung giờ: {new Date(detail.conversationSession.startedAtMs).toLocaleString("vi-VN")} – {new Date(detail.conversationSession.endsAtMs).toLocaleTimeString("vi-VN")}</strong>
        <p>{Math.max(0,Math.ceil((detail.conversationSession.endsAtMs-Date.now())/60000))} phút còn lại · {detail.assignments.filter(a=>a.state==="succeeded").length}/{detail.assignments.length} câu đã xác nhận</p>
        <p>Lượt tiếp sớm nhất: {new Date(detail.conversationSession.nextAtMs).toLocaleTimeString("vi-VN")} · luân phiên giữa các bài</p>
      </section>}
      {/* Above the threads on purpose: it is what the comments below were written from, so
          reading it first is reading the evidence before the verdict. */}
      <TargetNotesPanel notes={notes} />

      {/* Grouped by link, which is also grouped by team: `plan_threads` gives each cohort its
          own links, so one heading is one conversation on one post. A flat list of sixty rows
          from six teams running at once cannot be read. */}
      {Object.entries(byLink).map(([targetKey, rows], targetIndex) => (
        <div key={targetKey} className="interaction-thread">
          <div className="interaction-thread-head">
            <strong>{notes.find((note) => note.targetKey === targetKey)?.normalizedUrl
              ? <a href={notes.find((note) => note.targetKey === targetKey)!.normalizedUrl} target="_blank" rel="noreferrer">Bài {targetIndex + 1}</a>
              : `Bài ${targetIndex + 1}`}</strong>
            <details className="interaction-raw-code"><summary>Mã bài</summary><code>{targetKey}</code></details>
            <small>
              {rows.filter((row) => row.state === "succeeded").length}/{rows.length} lượt
            </small>
          </div>
          {rows.map((assignment) => {
            const scriptStep = detail.scriptedConversation?.targetScripts.find(s=>s.targetKey===targetKey)?.steps[assignment.ordinal];
            const reason = assignmentReason(assignment);
            const shotRecord = artifacts.find(
              (item) => item.assignmentId === assignment.id && item.relativePath,
            );
            // Only on a message that actually stopped, and only once the campaign has. The
            // backend would also accept `queued`/`preparing`/`ready`, but offering a retry
            // beside every message still waiting its turn puts a button on thirteen rows
            // that have not failed — and pressing one mid-run asks the engine to re-plan a
            // campaign that is still working through the first plan.
            const retryable =
              ["failed", "skippedParent"].includes(assignment.state) &&
              summary.state !== "running";
            return (
              <div key={assignment.id} className="interaction-assignment">
                <span>#{assignment.ordinal + 1}</span>
                <span className="grow">
                  <strong>{actorLabel(assignment.actorUdid)}{scriptStep && <small>{scriptStep.topic} · Vai {scriptStep.speakerId}</small>}</strong>
                  <details
                    className="interaction-raw-code"
                    aria-label={`Chi tiết kỹ thuật ${actorLabel(assignment.actorUdid)}`}
                  >
                    <summary>Thiết bị kỹ thuật</summary>
                    <code>{assignment.actorUdid}</code>
                  </details>
                  {(assignment.preparedText || !assignment.actions?.length) && (
                    <small>{assignment.preparedText ?? "Chưa chuẩn bị"}</small>
                  )}
                  {Boolean(assignment.actions?.length) && (
                    <div className="interaction-action-results" aria-label="Kết quả hành động">
                      {assignment.actions!.map((action) => (
                        <div key={action.kind} className="interaction-action-result">
                          <span className={`chip ${actionView(action, assignment).tone}`}>
                            {ACTION_KIND_VI[action.kind]} · {actionView(action, assignment).label}
                          </span>
                          {(action.error || action.evidence || isBlockedAction(assignment, action)) && (
                            <details
                              className="interaction-raw-code"
                              aria-label={`Chi tiết ${ACTION_KIND_VI[action.kind]}`}
                            >
                              <summary>Chi tiết</summary>
                              <code>{isBlockedAction(assignment, action) ? JSON.stringify(action) : action.error ?? action.evidence}</code>
                            </details>
                          )}
                          {(action.kind === "like" || action.kind === "save") && (
                            <PublicCleanupControl
                              campaignId={summary.id}
                              assignmentId={assignment.id}
                              targetKey={assignment.targetKey}
                              actorLabel={actorLabel(assignment.actorUdid)}
                              kind={action.kind}
                              sourceState={action.state}
                              sourceEvidence={action.evidence}
                            />
                          )}
                        </div>
                      ))}
                    </div>
                  )}
                  {reason && <Reason code={reason} />}
                  {assignment.errorCode && reason !== assignment.errorCode && (
                    <details className="interaction-raw-code" aria-label="Mã lỗi lượt gốc">
                      <summary>Mã lỗi lượt</summary>
                      <code>{assignment.errorCode}</code>
                    </details>
                  )}
                  {assignment.like && (
                    <small
                      className={
                        assignment.like.startsWith("đã tim") ? "hint" : "interaction-error"
                      }
                    >
                      {assignment.like}
                    </small>
                  )}
                  {/* A tag that stayed literal is not a failure — the comment posted — but it
                      is not what was asked for either, so it reads as a warning rather than
                      as a plain note. */}
                  {assignment.mention && (
                    <small
                      className={
                        assignment.mention.includes("chỉ là chữ") ? "interaction-error" : "hint"
                      }
                    >
                      {assignment.mention}
                    </small>
                  )}
                  {assignment.parentWasFolded && (
                    <small className="interaction-error">
                      Bình luận cha bị TikTok gấp; phản hồi này đã gửi nhưng người khác không nhìn
                      thấy.
                    </small>
                  )}
                  {assignment.state === "uncertain" && <InteractionReadbackControl campaignId={summary.id} assignmentId={assignment.id} disabled={busy || summary.state === "running"} />}
                </span>
                {shotRecord && (
                  <button
                    type="button"
                    className="ghost"
                    onClick={() => onShowShot(shotRecord.id)}
                  >
                    Ảnh
                  </button>
                )}
                {/* Per-message retry. The backend has taken `assignmentIds` since the feature
                    shipped and the UI only ever asked for all of them, so repairing one dead
                    phone meant re-running every retryable message on the campaign. */}
                {retryable && (
                  <button
                    type="button"
                    className="ghost"
                    disabled={busy}
                    onClick={() => onRetry([assignment.id])}
                  >
                    Thử lại
                  </button>
                )}
                <span className={`chip ${stateTone(assignment.state)}`}>
                  {assignmentStateVi(assignment.state)}
                </span>
              </div>
            );
          })}
        </div>
      ))}
      {shot && (
        <button type="button" className="interaction-shot" onClick={onDismissShot}>
          <img src={shot} alt="Ảnh màn hình khay bình luận" />
        </button>
      )}
  </>;
  return (
    <div className={`interaction-detail${compact ? " iw-monitor-result" : ""}`}>
      {!compact && <button type="button" className="ghost interaction-back" onClick={onBack}>
        ← Chiến dịch gần đây
      </button>}
      {error && <Banner tone="error">{error}</Banner>}
      {evidenceError && <Banner tone="error" action={<button type="button" className="ghost" onClick={onRetryEvidence}>Tải lại bằng chứng</button>}>{evidenceError}</Banner>}
      <div className="interaction-detail-head">
        <span className={`chip ${stateTone(summary.state)}`}>
          {campaignStateVi(summary.state)}
        </span>
        {detail.actionAggregate && (
          <span className={`chip ${actionAggregateTone(detail.actionAggregate)}`}>
            {actionAggregateVi(detail.actionAggregate)}
          </span>
        )}
        <small>
          {hasActionCounters
            ? `${actionSettled}/${actionCounters!.planned} hành động đã có kết quả`
            : `${summary.succeededMessages}/${total} bình luận`}
          {summary.failedMessages > 0 && ` · ${summary.failedMessages} lỗi`}
          {summary.updatedAt && ` · ${timeAgoVi(summary.updatedAt)}`}
        </small>
        {summary.state === "running" && (
          <button type="button" className="danger" disabled={busy} onClick={onCancel}>
            Dừng
          </button>
        )}
        {/* Offered only on a campaign that has finished badly: `Sending`, `Succeeded` and
            `Uncertain` assignments are excluded server-side because re-sending a comment that
            may already be public is the one thing this must never do. */}
        {["partial", "failed", "cancelled"].includes(summary.state) && detail.assignments.some((assignment) =>
          !["sending", "succeeded", "uncertain"].includes(assignment.state)) && (
          <button type="button" disabled={busy} onClick={() => onRetry()}>
            Thử lại phần hỏng
          </button>
        )}
      </div>
      {hasActionCounters && (
        <ActionCounters
          counters={actionCounters!}
          failedBeforeEffect={actionFailedBeforeEffect}
        />
      )}
      <ProgressBar
        fraction={
          hasActionCounters
            ? actionSettled / actionCounters!.planned
            : total > 0
              ? settled / total
              : null
        }
        failedFraction={
          hasActionCounters
            ? actionFailedBeforeEffect / actionCounters!.planned
            : total > 0
              ? summary.failedMessages / total
              : 0
        }
        tone={summary.state === "running" ? "run" : stateTone(summary.state) === "ok" ? "done" : "failed"}
        label="Tiến trình chiến dịch đang xem"
      />
      {summary.errorCode && <Reason code={summary.errorCode} />}

      {compact ? <>
        <InteractionMachineResults rows={byActor} actorLabel={actorLabel} onOpen={setSelectedActor} />
        <DetailDrawer open={selectedActor !== null} title={selectedActor ? actorLabel(selectedActor) : ""}
          onClose={() => { setSelectedActor(null); onDismissShot(); }}>
          {evidenceContent}
        </DetailDrawer>
      </> : evidenceContent}
    </div>
  );
}

function InteractionMachineResults({ rows, actorLabel, onOpen }: {
  rows: Map<string, InteractionAssignmentRecord[]>;
  actorLabel: (udid: string) => string;
  onOpen: (udid: string) => void;
}) {
  return <div className="iw-table-scroll iw-monitor-table-scroll" tabIndex={0} aria-label="Kết quả từng máy">
    <table className="iw-table iw-monitor-table"><thead><tr><th>Máy / tài khoản</th><th>Hành động</th><th>Kết quả</th><th>Chi tiết</th></tr></thead><tbody>
      {[...rows].map(([udid, assignments]) => {
        const results = assignments.flatMap((assignment) => (assignment.actions ?? []).map((action) => ({ action, assignment })));
        const counts = new Map<string, { label: string; tone: string; count: number }>();
        for (const { action, assignment } of results) {
          const view = actionView(action, assignment);
          const key = `${action.kind}:${view.label}`;
          const entry = counts.get(key) ?? { ...view, label: `${ACTION_KIND_VI[action.kind]} · ${view.label}`, count: 0 };
          entry.count++;
          counts.set(key, entry);
        }
        const targetCount = new Set(assignments.map((assignment) => assignment.targetKey)).size;
        return <tr key={udid}><td><strong>{actorLabel(udid)}</strong><small>{targetCount} bài · {assignments.length} lượt</small></td>
          <td>{[...new Set(results.map(({ action }) => ACTION_KIND_VI[action.kind]))].join(" → ") || "Bình luận"}</td>
          <td><div className="iw-machine-outcomes">{counts.size ? [...counts].map(([key, result]) => <span key={key} className={`chip ${result.tone}`}>{result.label}{result.count > 1 ? ` (${result.count})` : ""}</span>) : [...new Set(assignments.map((assignment) => assignment.state))].map((state) => <span key={state} className={`chip ${stateTone(state)}`}>{assignmentStateVi(state)}</span>)}</div></td>
          <td><button type="button" className="ghost" aria-label={`Xem log ${actorLabel(udid)}`} onClick={() => onOpen(udid)}>Xem log</button></td>
        </tr>;
      })}
    </tbody></table>
  </div>;
}
