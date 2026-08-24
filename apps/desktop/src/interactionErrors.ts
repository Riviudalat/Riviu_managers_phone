/**
 * Vietnamese for the codes an interaction campaign records against itself.
 *
 * The Monitor tab used to print these raw, so the most diagnostic string on screen was also
 * the least readable one: `parent_identity_not_confirmed_at_ordinal_3` and
 * `ai_comment_unavailable: ordinal 0 — comment_context_rejected: context=0 overall=0` sat in
 * a panel whose every other label was Vietnamese. There was a translation table for the
 * eleven *states* and none for the forty-odd *reasons*.
 *
 * Three things this deliberately does not do:
 *
 * - **It never throws the raw code away.** Every result carries it, and the UI keeps it
 *   behind a disclosure. The codes are what a bug report is written from, and the operator
 *   is not the only reader of this panel.
 * - **It never invents a translation for something it does not recognise.** The
 *   campaign-level `errorCode` is a free-text anyhow chain — much of it already Vietnamese,
 *   written by the engine — so an unknown string is passed through as the title rather than
 *   replaced by a guess.
 * - **It does not re-translate the detail.** The hierarchy refusals are stored by the engine
 *   as `code: câu tiếng Việt`, and that sentence is better than anything this file could
 *   write: it was authored next to the measurement it describes. The code becomes the title,
 *   the engine's sentence becomes the detail.
 */

export interface InteractionErrorView {
  /** Short Vietnamese headline for the failure. */
  title: string;
  /** Whatever the backend appended after the code, when it appended anything. */
  detail?: string;
  /** The original string, always, for diagnosis. */
  raw: string;
}

/**
 * Arrival and reply refusals from the hierarchy driver.
 *
 * These are the ones stored as `code: message`, so the map only has to name the code — see
 * the note about not re-translating the detail.
 */
const EXACT: Record<string, string> = {
  // Arrival: did the phone reach the post we meant?
  target_open_wrong_app: "Link mở sang app khác",
  target_open_no_post_page: "Không thấy trang bài viết",
  target_open_screen_unchanged: "Mở link xong màn hình không đổi",
  target_open_no_measured_label: "Bản TikTok này chưa được đo nhãn",
  target_open_no_baseline: "Không đọc được bài đang mở trước đó",
  target_open_cancelled: "Đã dừng khi đang mở bài",
  // Reply: did we find the right comment to answer?
  reply_control_unmeasured: "Chưa đo nút Trả lời trên bản này",
  reply_no_drawer: "Không mở được khay bình luận",
  reply_parent_not_found: "Không tìm thấy bình luận cha",
  reply_drawer_closed_by_scroll: "Cuộn làm đóng mất khay bình luận",
  reply_no_composer: "Không thấy ô nhập trả lời",
  reply_wrong_parent: "Bấm nhầm nút Trả lời của bình luận khác",
};

/**
 * The planner's own refusals, which reach the panel in English.
 *
 * `plan_threads` runs `ThreadCampaignRequest::validate()` and
 * `interaction_commands.rs` wraps whatever it says as
 * `CommandError::code("InteractionFailed", <the Display impl>)`. `describeError` keeps named
 * codes, so `planError` and `runError` arrived as e.g. `InteractionFailed: message count must
 * cover every selected actor` and went straight into a panel whose entire premise is that raw
 * codes never reach the operator. Keyed on the English sentence rather than on a code because
 * that sentence is all the wire carries — `thiserror` renders the variant, not its name.
 */
const PLANNER: Record<string, string> = {
  "request id is empty": "Thiếu mã yêu cầu",
  "at least one target is required": "Cần ít nhất một link",
  "message count must be between two and sixty-four":
    "Số bình luận mỗi link phải từ 2 đến 64",
  "actor count must be between two and sixty-four, and every actor distinct":
    "Số máy phải từ 2 đến 64 và không trùng nhau",
  "a cohort needs at least two actors": "Mỗi cụm cần ít nhất hai máy",
  "message count must cover every selected actor":
    "Số bình luận phải đủ cho cụm lớn nhất",
  "duplicate actor": "Một máy được chọn hai lần",
  "duplicate target": "Một link được dán hai lần",
  "comment length must be between four and twenty words":
    "Số từ mỗi câu phải từ 4 đến 20",
  "a manual comment is empty": "Có một câu bình luận để trống",
  "manual mode needs at least as many comments as there are messages":
    "Danh sách bình luận ít hơn số bình luận cần gửi",
};

/** Families, so a refusal added to the engine later still reads as something. */
const FAMILIES: [string, string][] = [
  ["target_open_", "Không mở được bài"],
  ["reply_", "Không trả lời được"],
];

/** One segment of a recorded failure, translated, or `undefined` if it names nothing known. */
function titleOf(segment: string): string | undefined {
  const exact = EXACT[segment];
  if (exact) return exact;

  const planner = PLANNER[segment];
  if (planner) return planner;

  // A skipped reply names the message it was waiting on. The ordinal is zero-based in the code
  // and one-based everywhere the operator reads it, which is why this is not a plain string
  // swap. Bounded to three digits: `message_count` is a `u8` capped at 64, and an unbounded
  // `Number` past 2^53 makes the `+ 1` a silent no-op.
  const skipped = /^parent_identity_not_confirmed_at_ordinal_(\d{1,3})$/.exec(segment);
  if (skipped) {
    const parent = Number(skipped[1]) + 1;
    return `Bỏ qua — bình luận thứ ${parent} chưa được xác nhận nên không có gì để trả lời`;
  }

  if (segment === "ai_comment_unavailable") return "AI không viết được bình luận";
  if (segment === "target_evidence_unavailable") return "Không chụp được bài cho AI đọc";

  for (const [prefix, title] of FAMILIES) {
    if (segment.startsWith(prefix)) return title;
  }
  return undefined;
}

/**
 * Translate one recorded failure.
 *
 * **Every `": "` segment is tried, not just the first.** `interaction_commands.rs` records the
 * campaign-level reason as `format!("{error:#}")`, and an anyhow chain renders outermost-first
 * — so the first segment is context like `AI chuẩn bị assignment 0` and the code sits one or
 * two segments in. Splitting only at the head meant the same failure printed two different ways
 * in one panel: the assignment row read "AI không viết được bình luận" while the campaign row
 * above it printed the raw chain, `<details>` and all suppressed because title equalled raw.
 */
export function interactionErrorVi(raw: string): InteractionErrorView {
  const trimmed = raw.trim();
  if (!trimmed) return { title: "Lỗi không rõ", raw };

  const parts = trimmed.split(": ");
  for (let at = 0; at < parts.length; at += 1) {
    const title = titleOf(parts[at].trim());
    if (!title) continue;
    // What the chain said around the code is kept as the detail: the context in front of it
    // names which assignment failed, and the tail is usually the engine's own sentence.
    const context = parts.slice(0, at).join(": ").trim();
    const tail = parts.slice(at + 1).join(": ").trim();
    const detail = [context, tail].filter((part) => part.length > 0).join(" — ");
    return { title, detail: detail || undefined, raw };
  }

  // Unknown: the campaign-level reason is free text and often already Vietnamese. Showing it
  // beats replacing it with "Lỗi".
  return { title: trimmed, raw };
}

/** Why one pasted line is not a usable link. */
const LINK_ERRORS: Record<string, string> = {
  empty: "Dòng trống",
  invalidUrl: "Không phải URL hợp lệ",
  unsupportedScheme: "Chỉ nhận link http/https",
  unsupportedHost: "Không phải link TikTok",
  userInfoNotAllowed: "Link có kèm thông tin đăng nhập",
  customPortNotAllowed: "Link dùng cổng lạ",
  unsupportedTargetKind: "Chỉ nhận link video hoặc ảnh",
  unresolvedShortLink: "Link rút gọn — bấm “Gỡ link rút gọn”",
};

export function linkErrorVi(code: string | null | undefined): string {
  if (!code) return "Không dùng được";
  return LINK_ERRORS[code] ?? code;
}

/** Campaign-level state. */
const CAMPAIGN_STATES: Record<string, string> = {
  queued: "Đang chờ",
  running: "Đang chạy",
  succeeded: "Hoàn tất",
  partial: "Xong một phần",
  failed: "Thất bại",
  cancelled: "Đã dừng",
};

export function campaignStateVi(state: string): string {
  return CAMPAIGN_STATES[state] ?? state;
}

/**
 * Per-message state.
 *
 * `uncertain` and `skippedParent` are spelled out rather than both reading "Chưa xác nhận":
 * one means the Send tap went out and its confirmation did not come back — so the comment may
 * be public and it will never be retried — and the other means nothing was typed at all.
 * Those need different reactions from the operator.
 */
const MESSAGE_STATES: Record<string, string> = {
  queued: "Đang chờ",
  preparing: "Đang soạn",
  ready: "Đã soạn",
  sending: "Đang gửi",
  succeeded: "Đã gửi",
  failed: "Lỗi",
  uncertain: "Đã gửi, chưa thấy lên",
  skippedParent: "Bỏ qua — thiếu bình luận cha",
};

export function assignmentStateVi(state: string): string {
  return MESSAGE_STATES[state] ?? state;
}

/** Chip colour for a state, in the tones the rest of the app uses. */
export function stateTone(state: string): "ok" | "warn" | "danger" | "info" {
  switch (state) {
    case "succeeded":
      return "ok";
    case "partial":
    case "uncertain":
    case "skippedParent":
      return "warn";
    // `failed` used to render in the neutral tone, so the one row the operator had to look
    // at was the one that looked like every other row.
    case "failed":
      return "danger";
    default:
      return "info";
  }
}
