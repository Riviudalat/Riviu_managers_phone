import { ProgressBar } from "../ProgressBar";
import { Banner } from "../States";
import {
  assignmentStateVi,
  campaignStateVi,
  interactionErrorVi,
  stateTone,
} from "../../interactionErrors";
import { timeAgoVi } from "../../timeAgo";
import type { InteractionArtifactRecord } from "../../api";
import type {
  DeviceInfo,
  InteractionCampaignDetail,
  InteractionTargetNote,
} from "../../types";

/** One recorded reason, in Vietnamese, with the code kept for whoever needs it. */
function Reason({ code }: { code: string }) {
  const view = interactionErrorVi(code);
  return (
    <span className="interaction-error">
      <strong>{view.title}</strong>
      {view.detail && <small>{view.detail}</small>}
      {/* Never thrown away. The Vietnamese is for the operator; the code is what a bug
          report is written from, and this panel has both kinds of reader. */}
      {view.title !== view.raw && (
        <details className="interaction-raw-code">
          <summary>mã lỗi</summary>
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
            <th>Caption</th>
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
  handles,
  busy,
  error,
  onBack,
  onCancel,
  onRetry,
  onShowShot,
  shot,
  onDismissShot,
}: {
  detail: InteractionCampaignDetail;
  artifacts: InteractionArtifactRecord[];
  notes: InteractionTargetNote[];
  devices: DeviceInfo[];
  deviceNumber: Map<string, number>;
  handles: Record<string, string>;
  busy: boolean;
  error: string | null;
  onBack: () => void;
  onCancel: () => void;
  onRetry: (assignmentIds?: string[]) => void;
  onShowShot: (artifactId: string) => void;
  shot: string | null;
  onDismissShot: () => void;
}) {
  const { summary } = detail;
  const total = summary.messageCount * summary.targetCount;
  const settled = summary.succeededMessages + summary.failedMessages;

  /// A phone by the number on its tile, not by eight characters of its udid.
  ///
  /// The shortening only applies to a udid long enough to need it: an iOS udid is 40
  /// characters, but an Android serial can be shorter than the cut, and truncating
  /// `android-1` to `android-` removes the only part that identifies it.
  const shortUdid = (udid: string) => (udid.length > 12 ? `${udid.slice(0, 8)}…` : udid);
  const actorLabel = (udid: string) => {
    const number = deviceNumber.get(udid);
    const device = devices.find((entry) => entry.udid === udid);
    const handle = handles[udid];
    if (!device && number === undefined) {
      // Ran on a phone that has since left the fleet. Saying so beats a bare udid that looks
      // like a phone the operator should be able to find on the wall.
      return `${shortUdid(udid)} (đã rời fleet)`;
    }
    const name = device?.name || device?.model || shortUdid(udid);
    return `${number ? `${number} · ` : ""}${name}${handle ? ` · @${handle}` : ""}`;
  };

  const byLink = detail.assignments.reduce<Record<string, typeof detail.assignments>>(
    (groups, assignment) => {
      (groups[assignment.targetKey] ??= []).push(assignment);
      return groups;
    },
    {},
  );

  return (
    <div className="interaction-detail">
      <button type="button" className="ghost interaction-back" onClick={onBack}>
        ← Chiến dịch gần đây
      </button>
      {error && <Banner tone="error">{error}</Banner>}
      <div className="interaction-detail-head">
        <span className={`chip ${stateTone(summary.state)}`}>
          {campaignStateVi(summary.state)}
        </span>
        <small>
          {summary.succeededMessages}/{total} bình luận
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
        {["partial", "failed", "cancelled"].includes(summary.state) && (
          <button type="button" disabled={busy} onClick={() => onRetry()}>
            Thử lại phần hỏng
          </button>
        )}
      </div>
      <ProgressBar
        fraction={total > 0 ? settled / total : null}
        failedFraction={total > 0 ? summary.failedMessages / total : 0}
        tone={summary.state === "running" ? "run" : stateTone(summary.state) === "ok" ? "done" : "failed"}
        label={`Tiến trình chiến dịch ${summary.id}`}
      />
      {summary.errorCode && <Reason code={summary.errorCode} />}

      {/* Above the threads on purpose: it is what the comments below were written from, so
          reading it first is reading the evidence before the verdict. */}
      <TargetNotesPanel notes={notes} />

      {/* Grouped by link, which is also grouped by team: `plan_threads` gives each cohort its
          own links, so one heading is one conversation on one post. A flat list of sixty rows
          from six teams running at once cannot be read. */}
      {Object.entries(byLink).map(([targetKey, rows]) => (
        <div key={targetKey} className="interaction-thread">
          <div className="interaction-thread-head">
            <strong>{targetKey.replace(/^content:/, "link ")}</strong>
            <small>
              {rows.filter((row) => row.state === "succeeded").length}/{rows.length} message
            </small>
          </div>
          {rows.map((assignment) => {
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
                  <strong>{actorLabel(assignment.actorUdid)}</strong>
                  <small>{assignment.preparedText ?? "Chưa chuẩn bị"}</small>
                  {assignment.errorCode && <Reason code={assignment.errorCode} />}
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
    </div>
  );
}
