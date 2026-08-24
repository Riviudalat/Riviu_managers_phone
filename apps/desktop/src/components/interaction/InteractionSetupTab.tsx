import { useState } from "react";
import { InfoDot as Info } from "../InfoDot";
import { Banner } from "../States";
import { InteractionActorPicker } from "./InteractionActorPicker";
import { InteractionPlanPreview } from "./InteractionPlanPreview";
import { linkErrorVi } from "../../interactionErrors";
import {
  effectiveMessageCount,
  manualCommentsOf,
  type DraftIssue,
  type InteractionDraft,
  type ThreadKind,
} from "../../interactionPlan";
import type { DeviceInfo, ThreadPreview, TikTokLinkLine } from "../../types";

/** The one decision that used to be two dependent dropdowns. */
const THREAD_KINDS: { value: ThreadKind; label: string; hint: string }[] = [
  {
    value: "star",
    label: "Toả",
    hint: "các máy cùng trả lời bình luận gốc, chạy song song",
  },
  {
    value: "chain",
    label: "Nối tiếp",
    hint: "máy sau trả lời máy trước, chạy lần lượt",
  },
  {
    value: "standalone",
    label: "Riêng lẻ",
    hint: "mỗi máy một bình luận gốc, không trả lời ai",
  },
];

export function InteractionSetupTab({
  draft,
  patch,
  lines,
  preview,
  issues,
  warnings,
  devices,
  deviceNumber,
  pixelActors,
  hierarchyActors,
  largestCohort,
  handles,
  onHandleChange,
  onHandleBlur,
  mentions,
  mentionActorCount,
  linkBusy,
  linkError,
  runError,
  busy,
  onResolveShortLinks,
  onRun,
}: {
  draft: InteractionDraft;
  patch: <K extends keyof InteractionDraft>(key: K, value: InteractionDraft[K]) => void;
  lines: TikTokLinkLine[];
  preview: ThreadPreview | null;
  issues: DraftIssue[];
  /** Advice, not refusals — these never disable the run button. */
  warnings: string[];
  devices: DeviceInfo[];
  deviceNumber: Map<string, number>;
  pixelActors: DeviceInfo[];
  hierarchyActors: DeviceInfo[];
  largestCohort: number;
  handles: Record<string, string>;
  onHandleChange: (udid: string, value: string) => void;
  onHandleBlur: (udid: string, value: string) => void;
  mentions: string[];
  mentionActorCount: number;
  linkBusy: boolean;
  linkError: string | null;
  runError: string | null;
  busy: boolean;
  onResolveShortLinks: () => void;
  onRun: () => void;
}) {
  // Closed by default. Three numbers that mostly want their defaults were the widest part of
  // a form the operator had to scroll four screens of.
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const messages = effectiveMessageCount(draft, largestCohort);
  const manualCount = manualCommentsOf(draft).length;

  return (
    <div className="interaction-body nu-pane">
      <div className="nu-group-head">Bài viết</div>
      <label className="nu-field">
        <span className="nu-label">Link TikTok — mỗi dòng một link</span>
        <textarea
          value={draft.rawLinks}
          onChange={(event) => patch("rawLinks", event.target.value)}
          placeholder="https://www.tiktok.com/@creator/video/123"
          rows={4}
        />
      </label>
      {linkError && <Banner tone="error">{linkError}</Banner>}
      <div className="interaction-link-list">
        {lines.map((line) => (
          <div key={line.lineNo} className={line.target ? "ok" : "bad"}>
            <span>{line.target ? "✓" : "!"}</span>
            <span>
              {line.target?.normalizedUrl ?? `${line.original} · ${linkErrorVi(line.error)}`}
            </span>
          </div>
        ))}
      </div>
      {lines.some((line) => line.error === "unresolvedShortLink") && (
        <button
          type="button"
          className="ghost"
          disabled={linkBusy}
          onClick={onResolveShortLinks}
        >
          Gỡ link rút gọn
        </button>
      )}

      <div className="nu-group-head">
        Kiểu tương tác
        <Info
          of="Kiểu tương tác"
          what="Toả = mọi acc trả lời bình luận gốc, chạy song song nên một rep hỏng chỉ mất chính nó. Nối tiếp = acc N trả lời acc N-1, chạy nối đuôi nên một mắt xích đứt là dừng cả link. Riêng lẻ = mỗi acc một bình luận gốc, chạy được mọi máy và trộn iPhone + Android được."
        />
      </div>
      <div className="interaction-mode-cards" role="radiogroup" aria-label="Kiểu tương tác">
        {THREAD_KINDS.map((kind) => (
          <label
            key={kind.value}
            className={`interaction-mode-card${draft.threadKind === kind.value ? " selected" : ""}`}
          >
            <input
              type="radio"
              name="thread-kind"
              className="tile-check"
              value={kind.value}
              checked={draft.threadKind === kind.value}
              onChange={() => patch("threadKind", kind.value)}
            />
            <strong>{kind.label}</strong>
            <small>{kind.hint}</small>
          </label>
        ))}
      </div>
      <label className="nu-switch">
        <input
          type="checkbox"
          checked={draft.likeTarget}
          onChange={(event) => patch("likeTarget", event.target.checked)}
          aria-label="Thả tim bài"
        />
        <span className="nu-switch-track" aria-hidden="true" />
        <span className="nu-switch-label">
          Thả tim bài
          <Info
            of="Thả tim bài"
            what="Mỗi actor thả tim bài trước khi bình luận, xác nhận bằng nhãn nút tim đổi trạng thái. Android làm được; iPhone bị từ chối vì chưa đo toạ độ nút tim. Thả tim hỏng không làm mất bình luận."
          />
        </span>
      </label>

      <div className="nu-group-head">Nội dung</div>
      <label className="nu-field">
        <span className="nu-label">Nội dung bình luận</span>
        <select
          value={draft.textSource}
          onChange={(event) => patch("textSource", event.target.value as "ai" | "manual")}
        >
          <option value="ai">AI viết — đọc bài rồi tự soạn</option>
          <option value="manual">Thủ công — dán sẵn danh sách</option>
        </select>
      </label>
      {draft.textSource === "ai" ? (
        <label className="nu-field">
          <span className="nu-label">Hướng dẫn giọng điệu cho AI</span>
          <input
            value={draft.instruction}
            onChange={(event) => patch("instruction", event.target.value)}
          />
        </label>
      ) : (
        <>
          <label className="nu-field">
            <span className="nu-label">
              Danh sách bình luận — mỗi dòng một câu
              <Info
                of="Danh sách bình luận"
                what="Chia lần lượt theo từng link nên nhiều link không mở đầu bằng cùng một câu; chạy lại cùng chiến dịch sẽ gửi đúng chữ đó. Cần ít nhất số câu bằng số bình luận mỗi link."
              />
            </span>
            <textarea
              rows={5}
              value={draft.manualText}
              placeholder={["đẹp quá", "chỗ này ở đâu vậy ạ", "lưu lại đi ăn thử"].join("\n")}
              onChange={(event) => patch("manualText", event.target.value)}
            />
          </label>
          <p className="hint">
            {manualCount} câu · cần ≥ {messages}
          </p>
        </>
      )}

      <div className="nu-group-head">Thiết bị &amp; tag</div>
      <InteractionActorPicker
        pixelActors={pixelActors}
        hierarchyActors={hierarchyActors}
        deviceNumber={deviceNumber}
        actors={draft.actors}
        onToggle={(udid) =>
          patch(
            "actors",
            draft.actors.includes(udid)
              ? draft.actors.filter((id) => id !== udid)
              : [...draft.actors, udid],
          )
        }
        onReplace={(udids) => patch("actors", udids)}
        handles={handles}
        onHandleChange={onHandleChange}
        onHandleBlur={onHandleBlur}
        mentionText={draft.mentionText}
        onMentionText={(value) => patch("mentionText", value)}
        mentions={mentions}
        mentionActorCount={mentionActorCount}
      />

      <button
        type="button"
        className="ghost interaction-advanced-toggle"
        aria-expanded={advancedOpen}
        onClick={() => setAdvancedOpen((open) => !open)}
      >
        {advancedOpen ? "Ẩn tuỳ chỉnh nâng cao" : "Tuỳ chỉnh nâng cao"}
      </button>
      {advancedOpen && (
        <div className="nu-grid interaction-grid-3">
          <label className="nu-field">
            <span className="nu-label">
              Số bình luận mỗi link
              {draft.messageCount === null && <span className="chip info">tự động</span>}
            </span>
            <input
              type="number"
              min={2}
              max={64}
              // Placeholder rather than value while it is on auto: showing the computed number
              // as a value would look like a choice the operator made, and typing over it
              // would then be the only way back to auto.
              placeholder={String(messages)}
              value={draft.messageCount ?? ""}
              onChange={(event) =>
                patch(
                  "messageCount",
                  event.target.value === "" ? null : Number(event.target.value),
                )
              }
            />
          </label>
          <label className="nu-field">
            <span className="nu-label">Số máy mỗi cụm</span>
            <input
              type="number"
              min={0}
              max={64}
              value={draft.cohortSize}
              onChange={(event) => patch("cohortSize", Number(event.target.value))}
            />
          </label>
          <label className="nu-field">
            <span className="nu-label">Số từ tối đa mỗi câu</span>
            <input
              type="number"
              min={4}
              max={20}
              value={draft.maxWords}
              onChange={(event) => patch("maxWords", Number(event.target.value))}
            />
          </label>
        </div>
      )}
      {advancedOpen && (
        <p className="hint">
          Số máy mỗi cụm = 0 nghĩa là cả nhóm chung một cụm, lần lượt từng máy. Để trống số
          bình luận là để app tự lấy bằng cụm lớn nhất.
        </p>
      )}

      <InteractionPlanPreview
        preview={preview}
        devices={devices}
        deviceNumber={deviceNumber}
        threadKind={draft.threadKind}
      />

      {warnings.map((warning) => (
        <Banner key={warning} tone="warn">
          {warning}
        </Banner>
      ))}
      {runError && <Banner tone="error">{runError}</Banner>}
      {issues.length > 0 && (
        <ul className="interaction-reasons">
          {issues.map((issue) => (
            <li key={`${issue.field}:${issue.message}`}>
              <span>{issue.message}</span>
              {issue.fix && (
                <button
                  type="button"
                  className="ghost"
                  onClick={() => patch("messageCount", issue.fix!.messageCount)}
                >
                  {issue.fix.label}
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
      <button
        type="button"
        className="primary interaction-run"
        disabled={busy || issues.length > 0}
        onClick={onRun}
      >
        Chạy ngay
      </button>
    </div>
  );
}
