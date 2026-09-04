import { InfoDot as Info } from "../InfoDot";
import { InteractionThreshold, type ThresholdControls } from "./InteractionThreshold";
import { Banner } from "../States";
import { InteractionActorPicker } from "./InteractionActorPicker";
import { InteractionPlanPreview } from "./InteractionPlanPreview";
import { linkErrorVi } from "../../interactionErrors";
import {
  effectiveMessageCount,
  manualCommentsOf,
  wholeNumber,
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
  threshold,
  advancedOpen,
  setAdvancedOpen,
  draft,
  patch,
  lines,
  preview,
  issues,
  warnings,
  devices,
  deviceNumber,
  deviceLabel,
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
  patch: <K extends keyof InteractionDraft>(
    key: K,
    value: InteractionDraft[K] | ((previous: InteractionDraft[K]) => InteractionDraft[K]),
  ) => void;
  lines: TikTokLinkLine[];
  preview: ThreadPreview | null;
  issues: DraftIssue[];
  /** Advice, not refusals — these never disable the run button. */
  warnings: string[];
  devices: DeviceInfo[];
  deviceNumber: Map<string, number>;
  deviceLabel: Map<string, string>;
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
  /** The threshold section's state, owned by the shell so a tab switch keeps it. */
  threshold: ThresholdControls;
  /**
   * Closed by default. Three numbers that mostly want their defaults were the widest part of a
   * form the operator had to scroll four screens of.
   *
   * Held by the shell rather than here, because it is state a tab switch has to survive — which
   * is what the shell's own docstring promises, and what this being a `useState` in a component
   * mounted per tab quietly broke: opening Nâng cao, checking the Monitor tab and coming back
   * collapsed it again.
   */
  advancedOpen: boolean;
  setAdvancedOpen: (open: boolean) => void;
}) {
  const messages = effectiveMessageCount(draft, largestCohort);
  const manualCount = manualCommentsOf(draft).length;
  const selectedActionCount = Object.values(draft.actions).filter(Boolean).length;
  const actionOrder = [
    draft.actions.like && "Tim",
    draft.actions.save && "Lưu",
    draft.actions.comment && "Bình luận",
  ].filter(Boolean);

  return (
    <div className="interaction-body nu-pane">
      <div className="nu-group-head">Bài viết</div>
      <label className="nu-field">
        <span className="nu-label">Link TikTok — mỗi dòng một link</span>
        <textarea
          value={draft.rawLinks}
          onChange={(event) => patch("rawLinks", event.target.value)}
          placeholder="Dán link TikTok, mỗi dòng một bài"
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

      <div className="nu-group-head">Hành động</div>
      <div className="interaction-action-controls" role="group" aria-label="Hành động">
        {([
          ["like", "Tim"],
          ["comment", "Bình luận"],
          ["save", "Lưu"],
        ] as const).map(([action, label]) => {
          const checked = draft.actions[action];
          return (
            <label key={action} className="nu-switch">
              <input
                type="checkbox"
                checked={checked}
                disabled={checked && selectedActionCount === 1}
                onChange={(event) =>
                  patch("actions", { ...draft.actions, [action]: event.target.checked })
                }
                aria-label={label}
              />
              <span className="nu-switch-track" aria-hidden="true" />
              <span className="nu-switch-label">{label}</span>
            </label>
          );
        })}
      </div>
      <p className="hint interaction-action-consequence">
        Thực hiện theo thứ tự: {actionOrder.join(" → ")}.
      </p>

      {draft.actions.comment && (
        <>
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
      {/* Only for a shape that has replies. `Riêng lẻ` gives every phone its own root
          comment and no parent, so there is nobody to tag and the switch would be a promise
          the run cannot keep. */}
      {draft.threadKind !== "standalone" && (
        <label className="nu-switch">
          <input
            type="checkbox"
            checked={draft.mentionParent}
            onChange={(event) => patch("mentionParent", event.target.checked)}
            aria-label="Các máy tag nhau khi trả lời"
          />
          <span className="nu-switch-track" aria-hidden="true" />
          <span className="nu-switch-label">
            Các máy tag nhau khi trả lời
            <Info
              of="Các máy tag nhau khi trả lời"
              what="Mỗi rep mở đầu bằng @handle của máy nó đang trả lời — Nối tiếp thì tag lần lượt xuống, Toả thì mọi rep đều tag máy mở đầu. Handle lấy từ ô @handle của từng máy ở dưới; máy chưa điền thì không bị tag chứ không tag bừa. Android chọn từ danh sách gợi ý của TikTok nên ra tag thật; iPhone chỉ chèn được chữ."
            />
          </span>
        </label>
      )}
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

      <InteractionThreshold controls={threshold} />
        </>
      )}

      <div className="nu-group-head">{draft.actions.comment ? "Thiết bị & tag" : "Thiết bị"}</div>
      <div className={draft.actions.comment ? undefined : "interaction-comment-free"}>
        <InteractionActorPicker
          pixelActors={pixelActors}
          hierarchyActors={hierarchyActors}
          deviceNumber={deviceNumber}
          deviceLabel={deviceLabel}
          commentEnabled={draft.actions.comment}
          threadsByGroup={draft.actions.comment && draft.threadKind !== "standalone"}
          actors={draft.actors}
          onToggle={(udid) =>
            // The updater form, not a value computed from the rendered prop: see `patch`.
            patch("actors", (previous) =>
              previous.includes(udid)
                ? previous.filter((id) => id !== udid)
                : [...previous, udid],
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
      </div>

      {draft.actions.comment && (
        <>
      <button
        type="button"
        className="ghost interaction-advanced-toggle"
        aria-expanded={advancedOpen}
        onClick={() => setAdvancedOpen(!advancedOpen)}
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
                  event.target.value === "" ? null : wholeNumber(event.target.value),
                )
              }
            />
          </label>
          <label className="nu-field">
            <span className="nu-label">Số từ tối đa mỗi câu</span>
            <input
              type="number"
              min={4}
              max={20}
              value={draft.maxWords}
              onChange={(event) => patch("maxWords", wholeNumber(event.target.value))}
            />
          </label>
        </div>
      )}
      {advancedOpen && (
        <p className="hint">
          Để trống số bình luận là để app tự lấy bằng số máy đã chọn.
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
        </>
      )}
      {runError && <Banner tone="error">{runError}</Banner>}
      {issues.length > 0 && (
        <ul className="interaction-reasons">
          {issues.map((issue) => (
            <li key={`${issue.field}:${issue.message}`}>
              <span>
                {issue.message}
                {issue.technicalDetail && (
                  <details className="interaction-raw-code" aria-label="Chi tiết lỗi lập kế hoạch">
                    <summary>Chi tiết kỹ thuật</summary>
                    <code>{issue.technicalDetail}</code>
                  </details>
                )}
              </span>
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
